use serde::Deserialize;
use std::collections::BTreeMap;

#[derive(Debug, Deserialize)]
struct ReleaseManifest {
    binary_name: String,
    display_name: String,
    ffmpeg_source: FfmpegSource,
    targets: BTreeMap<String, ReleaseTarget>,
}

#[derive(Debug, Deserialize)]
struct FfmpegSource {
    base_url: String,
    checksums_asset: String,
}

#[derive(Debug, Deserialize)]
struct ReleaseTarget {
    artifact_name: String,
    archive_format: String,
    bundle_dir: String,
    entry_executable: String,
    ffmpeg_asset: String,
    ffmpeg_sha256: String,
    rust_target: String,
}

fn manifest() -> ReleaseManifest {
    serde_json::from_str(include_str!("../tools/release_manifest.json"))
        .expect("valid release manifest")
}

#[test]
fn manifest_defines_linux_and_windows_targets() {
    let manifest = manifest();

    assert_eq!(manifest.binary_name, "observans");
    assert_eq!(manifest.display_name, "Observans");
    assert!(manifest
        .ffmpeg_source
        .base_url
        .contains("github.com/BtbN/FFmpeg-Builds"));
    assert_eq!(manifest.ffmpeg_source.checksums_asset, "checksums.sha256");

    let linux = manifest.targets.get("linux-x64").expect("linux target");
    assert_eq!(linux.artifact_name, "Observans-linux-x64.tar.gz");
    assert_eq!(linux.archive_format, "tar.gz");
    assert_eq!(linux.bundle_dir, "Observans-linux-x64");
    assert_eq!(linux.entry_executable, "observans");
    assert_eq!(linux.rust_target, "x86_64-unknown-linux-musl");
    assert_eq!(
        linux.ffmpeg_asset,
        "ffmpeg-master-latest-linux64-gpl.tar.xz"
    );
    assert!(!linux.ffmpeg_sha256.is_empty());

    let windows = manifest.targets.get("windows-x64").expect("windows target");
    assert_eq!(windows.artifact_name, "Observans-windows-x64.zip");
    assert_eq!(windows.archive_format, "zip");
    assert_eq!(windows.bundle_dir, "Observans-windows-x64");
    assert_eq!(windows.entry_executable, "observans.exe");
    assert_eq!(windows.rust_target, "x86_64-pc-windows-msvc");
    assert_eq!(windows.ffmpeg_asset, "ffmpeg-master-latest-win64-gpl.zip");
    assert!(!windows.ffmpeg_sha256.is_empty());
}

#[test]
fn linux_installer_block_matches_manifest() {
    let manifest = manifest();
    let linux = manifest.targets.get("linux-x64").expect("linux target");
    let install_script = include_str!("../install.sh");

    assert!(install_script.contains(&format!("ARTIFACT_LINUX_X64=\"{}\"", linux.artifact_name)));
    assert!(install_script.contains("FFMPEG_BIN_REL=\"ffmpeg/bin/ffmpeg\""));
}
