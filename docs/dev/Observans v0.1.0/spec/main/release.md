# Release

- Date:   **2026-04-07**
- Status: **Затверджено**
- Note:   **Актуалізований файл документації для поточного стану `release Observans v0.1.0`.**

## Джерела істини

Поточний release-контур описують і реалізують:

- [`tools/release_manifest.json`](../../../../../tools/release_manifest.json)
- [`tools/build_release_linux.sh`](../../../../../tools/build_release_linux.sh)
- [`tools/build_release_windows.ps1`](../../../../../tools/build_release_windows.ps1)
- [`.github/workflows/ci.yml`](../../../../../.github/workflows/ci.yml)
- [`.github/workflows/release.yml`](../../../../../.github/workflows/release.yml)
- [`tests/release_contracts.rs`](../../../../../tests/release_contracts.rs)

## Release targets

| Target | Rust target | Artifact |
| --- | --- | --- |
| `linux-x64` | `x86_64-unknown-linux-musl` | `Observans-linux-x64.tar.gz` |
| `windows-x64` | `x86_64-pc-windows-msvc` | `Observans-windows-x64.zip` |

## Пайплайн

### Linux

- `tools/build_release_linux.sh`
- `Observans.sh` як launcher
- runtime binary в `_observans_runtime/bin/observans`
- bundled FFmpeg в `_observans_runtime/ffmpeg/bin/ffmpeg`

### Windows

- `tools/build_release_windows.ps1`
- `observans.exe` в корені bundle
- bundled FFmpeg в `_observans_runtime/ffmpeg/bin/ffmpeg.exe`

## GitHub Actions

### `ci.yml`

- `cargo fmt --check`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test --workspace`

### `release.yml`

- build Linux bundle
- smoke-test Linux archive
- build Windows bundle
- smoke-test Windows archive
- refresh `rolling-main`
- publish rolling pre-release

## Див. також

- [README.md](README.md)
- [architecture.md](architecture.md)
- [crates.md](crates.md)
