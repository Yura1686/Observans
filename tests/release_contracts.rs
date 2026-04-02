use serde::Deserialize;
use std::collections::BTreeMap;

#[derive(Debug, Deserialize)]
struct ReleaseManifest {
    binary_name: String,
    display_name: String,
    ffmpeg_sources: BTreeMap<String, FfmpegSource>,
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
    launcher_kind: String,
    ffmpeg_source: String,
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
    assert_eq!(manifest.targets.len(), 2);
    let ffmpeg_source = manifest
        .ffmpeg_sources
        .get("btbn_latest")
        .expect("btbn_latest source");
    assert!(ffmpeg_source
        .base_url
        .contains("github.com/BtbN/FFmpeg-Builds"));
    assert_eq!(ffmpeg_source.checksums_asset, "checksums.sha256");

    let linux = manifest.targets.get("linux-x64").expect("linux target");
    assert_eq!(linux.artifact_name, "Observans-linux-x64.tar.gz");
    assert_eq!(linux.archive_format, "tar.gz");
    assert_eq!(linux.bundle_dir, "Observans-linux-x64");
    assert_eq!(linux.entry_executable, "observans");
    assert_eq!(linux.launcher_kind, "shell");
    assert_eq!(linux.ffmpeg_source, "btbn_latest");
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
    assert_eq!(windows.launcher_kind, "none");
    assert_eq!(windows.ffmpeg_source, "btbn_latest");
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

#[test]
fn release_readme_mentions_click_targets() {
    let readme = include_str!("../RELEASE_README.md");

    assert!(readme.contains("observans.exe"));
    assert!(readme.contains("Observans.sh"));
    assert!(!readme.contains("./observans"));
}

#[test]
fn release_workflow_uses_main_branch_rolling_release() {
    let workflow = include_str!("../.github/workflows/release.yml");

    assert!(workflow.contains("branches:"));
    assert!(workflow.contains("- main"));
    assert!(workflow.contains("rolling-main"));
    assert!(workflow.contains("gh release create rolling-main"));
    assert!(workflow.contains(
        "smoke/linux/Observans-linux-x64/_observans_runtime/bin/observans"
    ));
    assert!(!workflow.contains("tags:"));
    assert!(!workflow.contains(".sha256"));
}
