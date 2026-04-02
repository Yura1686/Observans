# Building Observans Releases

Observans has two release packaging entrypoints:

- `tools/build_release_linux.sh`
- `tools/build_release_windows.ps1`

## Official release strategy

- Linux release: statically-linked-as-practical musl binary inside `Observans-linux-x64.tar.gz`
- Windows release: `observans.exe` built for `x86_64-pc-windows-msvc` with static CRT
- Both bundles include `_observans_runtime/ffmpeg/bin/...`

## Linux

Prerequisites:

- Rust toolchain
- `python3`
- `tar`
- `sha256sum`
- `curl` or `wget`
- musl tooling for `x86_64-unknown-linux-musl`

Build:

```bash
bash tools/build_release_linux.sh
```

Outputs:

- `dist/Observans-linux-x64.tar.gz`
- `dist/Observans-linux-x64.tar.gz.sha256`
- `dist/install.sh`
- `dist/uninstall.sh`
- `dist/install.sh` is stamped with the release repository slug

## Windows

Run from PowerShell on Windows:

```powershell
./tools/build_release_windows.ps1
```

Outputs:

- `dist/Observans-windows-x64.zip`
- `dist/Observans-windows-x64.zip.sha256`

## Notes

- Bundled FFmpeg is resolved automatically at runtime when present next to the executable.
- `OBSERVANS_FFMPEG` still overrides bundled/runtime lookup.
- Official release builds are produced in GitHub Actions via `.github/workflows/release.yml`.
