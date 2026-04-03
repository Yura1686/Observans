# Building Observans Releases

This document covers release packaging for Observans. The project currently
ships Linux and Windows `x86_64` bundles only.

## Release Targets

| Target | Rust target | Archive |
| --- | --- | --- |
| Linux x64 | `x86_64-unknown-linux-musl` | `Observans-linux-x64.tar.gz` |
| Windows x64 | `x86_64-pc-windows-msvc` | `Observans-windows-x64.zip` |

Both bundles include:

- the Observans entrypoint for that platform
- bundled FFmpeg under `_observans_runtime/ffmpeg/bin`
- a release-facing `README.md`
- `_observans_runtime/build_meta.json`

## Packaging Entrypoints

Use the dedicated release scripts in `tools/`:

```bash
bash tools/build_release_linux.sh
```

```powershell
./tools/build_release_windows.ps1
```

The release matrix and FFmpeg source definitions are maintained in
`tools/release_manifest.json`.

## Linux Build

### Prerequisites

- Rust toolchain
- `python3`
- `tar`
- `sha256sum`
- `curl` or `wget`
- musl tooling for `x86_64-unknown-linux-musl`

### Output

Running the Linux packager produces:

- `dist/Observans-linux-x64.tar.gz`
- `dist/Observans-linux-x64.tar.gz.sha256`
- `dist/install.sh`
- `dist/uninstall.sh`

Archive structure:

- `Observans-linux-x64/Observans.sh`
- `Observans-linux-x64/_observans_runtime/bin/observans`
- `Observans-linux-x64/_observans_runtime/ffmpeg/bin/ffmpeg`
- `Observans-linux-x64/README.md`
- `Observans-linux-x64/_observans_runtime/build_meta.json`

Notes:

- `Observans.sh` is the user-facing launcher.
- The actual ELF binary lives under `_observans_runtime/bin/observans`.
- `dist/install.sh` is stamped with the release repository slug when packaging.

## Windows Build

Run the Windows packager from PowerShell on Windows:

```powershell
./tools/build_release_windows.ps1
```

### Output

- `dist/Observans-windows-x64.zip`
- `dist/Observans-windows-x64.zip.sha256`

Archive structure:

- `Observans-windows-x64/observans.exe`
- `Observans-windows-x64/_observans_runtime/ffmpeg/bin/ffmpeg.exe`
- `Observans-windows-x64/README.md`
- `Observans-windows-x64/_observans_runtime/build_meta.json`

## Bundled FFmpeg

Release scripts download FFmpeg from the source defined in
`tools/release_manifest.json`, verify its SHA-256 checksum, and copy the
runtime binaries into the bundle.

At runtime, Observans still honors the override order documented in the main
README:

1. `OBSERVANS_FFMPEG`
2. bundled runtime FFmpeg
3. `ffmpeg` from `PATH`

## CI Release Flow

GitHub Actions builds releases through `.github/workflows/release.yml`.

The workflow:

- builds Linux and Windows bundles
- smoke-tests both archives
- refreshes the `rolling-main` tag
- publishes one rolling pre-release from the latest successful `main` build

The rolling GitHub Release publishes only the two runnable archives:

- `Observans-linux-x64.tar.gz`
- `Observans-windows-x64.zip`

Checksum files and Linux installer helper scripts are generated locally during
packaging, but are not uploaded as release assets.
