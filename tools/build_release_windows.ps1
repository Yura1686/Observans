param(
    [string]$TargetId = "windows-x64"
)

$ErrorActionPreference = "Stop"

$RootDir = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$ManifestPath = Join-Path $RootDir "tools/release_manifest.json"
$DistDir = if ($env:OBSERVANS_DIST_DIR) { $env:OBSERVANS_DIST_DIR } else { Join-Path $RootDir "dist" }
$WorkDir = if ($env:OBSERVANS_WORK_DIR) { $env:OBSERVANS_WORK_DIR } else { Join-Path $RootDir ".release-work\$TargetId" }

function Require-Command {
    param([string]$Name)
    if (-not (Get-Command $Name -ErrorAction SilentlyContinue)) {
        throw "missing required command: $Name"
    }
}

function Get-ManifestConfig {
    param([string]$Manifest, [string]$ReleaseTarget)
    $payload = Get-Content -Raw -Path $Manifest | ConvertFrom-Json
    $target = $payload.targets.$ReleaseTarget
    [pscustomobject]@{
        DisplayName = $payload.display_name
        BinaryName = $payload.binary_name
        TargetOs = $target.os
        RustTarget = $target.rust_target
        ArtifactName = $target.artifact_name
        ArchiveFormat = $target.archive_format
        BundleDir = $target.bundle_dir
        EntryExecutable = $target.entry_executable
        FfmpegAsset = $target.ffmpeg_asset
        FfmpegSha256 = $target.ffmpeg_sha256
        FfmpegBaseUrl = $payload.ffmpeg_source.base_url
        FfmpegChecksumsAsset = $payload.ffmpeg_source.checksums_asset
    }
}

function Resolve-Checksum {
    param(
        [string]$ChecksumsPath,
        [string]$AssetName,
        [string]$Configured
    )

    if ($Configured -ne "auto") {
        return $Configured
    }

    foreach ($line in Get-Content -Path $ChecksumsPath) {
        $parts = $line.Trim() -split "\s+"
        if ($parts.Length -ge 2 -and $parts[-1] -eq $AssetName) {
            return $parts[0]
        }
    }

    throw "checksum for $AssetName not found"
}

function Write-BuildMeta {
    param(
        [string]$Destination,
        [object]$Config
    )

    $gitCommit = "unknown"
    try {
        $gitCommit = (git -C $RootDir rev-parse --short HEAD).Trim()
    } catch {
    }

    $payload = [ordered]@{
        display_name = $Config.DisplayName
        binary_name = $Config.BinaryName
        target_id = $TargetId
        target_os = $Config.TargetOs
        rust_target = $Config.RustTarget
        artifact_name = $Config.ArtifactName
        ffmpeg_asset = $Config.FfmpegAsset
        git_commit = $gitCommit
        built_at = (Get-Date).ToUniversalTime().ToString("yyyy-MM-ddTHH:mm:ssZ")
    }

    $payload | ConvertTo-Json | Set-Content -Path $Destination -Encoding utf8
}

Require-Command cargo
Require-Command Invoke-WebRequest

$config = Get-ManifestConfig -Manifest $ManifestPath -ReleaseTarget $TargetId
if ($config.TargetOs -ne "windows") {
    throw "target $TargetId is not a windows target"
}

New-Item -ItemType Directory -Force -Path $DistDir, (Join-Path $WorkDir "downloads"), (Join-Path $WorkDir "extracted") | Out-Null

if (Get-Command rustup -ErrorAction SilentlyContinue) {
    rustup target add $config.RustTarget | Out-Null
}

$previousRustFlags = $env:RUSTFLAGS
$env:RUSTFLAGS = "-C target-feature=+crt-static"
try {
    cargo build --release --target $config.RustTarget
} finally {
    $env:RUSTFLAGS = $previousRustFlags
}

$bundleDir = Join-Path $DistDir $config.BundleDir
$runtimeDir = Join-Path $bundleDir "_observans_runtime"
$bundleFfmpegDir = Join-Path $runtimeDir "ffmpeg/bin"
$buildMeta = Join-Path $runtimeDir "build_meta.json"
$binarySrc = Join-Path $RootDir "target\$($config.RustTarget)\release\$($config.BinaryName).exe"
$archivePath = Join-Path $DistDir $config.ArtifactName
$checksumPath = "$archivePath.sha256"
$ffmpegArchive = Join-Path $WorkDir "downloads\$($config.FfmpegAsset)"
$ffmpegChecksums = Join-Path $WorkDir "downloads\$($config.FfmpegChecksumsAsset)"
$extractedDir = Join-Path $WorkDir "extracted\$TargetId"

Remove-Item -Recurse -Force -ErrorAction SilentlyContinue $bundleDir, $extractedDir, $archivePath, $checksumPath
New-Item -ItemType Directory -Force -Path $bundleFfmpegDir, $extractedDir | Out-Null

Copy-Item -Path $binarySrc -Destination (Join-Path $bundleDir $config.EntryExecutable)
Copy-Item -Path (Join-Path $RootDir "RELEASE_README.md") -Destination (Join-Path $bundleDir "README.md")

Invoke-WebRequest -Uri "$($config.FfmpegBaseUrl)/$($config.FfmpegAsset)" -OutFile $ffmpegArchive
Invoke-WebRequest -Uri "$($config.FfmpegBaseUrl)/$($config.FfmpegChecksumsAsset)" -OutFile $ffmpegChecksums

$expectedChecksum = Resolve-Checksum -ChecksumsPath $ffmpegChecksums -AssetName $config.FfmpegAsset -Configured $config.FfmpegSha256
$actualChecksum = (Get-FileHash -Algorithm SHA256 -Path $ffmpegArchive).Hash.ToLowerInvariant()
if ($actualChecksum -ne $expectedChecksum.ToLowerInvariant()) {
    throw "checksum mismatch for $($config.FfmpegAsset)"
}

Expand-Archive -Path $ffmpegArchive -DestinationPath $extractedDir -Force
$ffmpegPath = Get-ChildItem -Path $extractedDir -Recurse -Filter ffmpeg.exe | Select-Object -First 1
if (-not $ffmpegPath) {
    throw "ffmpeg.exe not found in $($config.FfmpegAsset)"
}

Copy-Item -Path (Join-Path $ffmpegPath.Directory.FullName "*") -Destination $bundleFfmpegDir -Recurse -Force
Write-BuildMeta -Destination $buildMeta -Config $config

Compress-Archive -Path $bundleDir -DestinationPath $archivePath -CompressionLevel Optimal
$hashLine = "{0}  {1}" -f ((Get-FileHash -Algorithm SHA256 -Path $archivePath).Hash.ToLowerInvariant()), ([System.IO.Path]::GetFileName($archivePath))
Set-Content -Path $checksumPath -Value $hashLine -Encoding ascii

Write-Host "built $archivePath"
