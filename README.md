# Observans

Observans vNext is a cross-platform camera streaming workspace with:

- a startup CLI/TUI camera picker
- a local browser UI in the original twilight style
- MJPEG live streaming
- browser-side recording
- live telemetry for host and stream health

## Workspace

- `observans` root binary: bootstrap, startup, process wiring
- `observans-core`: config, platform detection, camera inventory, TUI bootstrap, metrics, capture runtime
- `observans-bus`: shared frame bus contract
- `observans-web`: Axum routes and embedded UI assets

## Prerequisites

- Rust toolchain with `cargo`
- `ffmpeg` available on `PATH` or via `OBSERVANS_FFMPEG`
- Linux: `v4l2-ctl` is optional but improves device listing

## Run

```bash
cargo run
```

Helpful flags:

```bash
cargo run -- --device auto --port 8080 --width 1280 --height 720 --fps 30
```

Notes:

- When the process starts inside an interactive terminal, Observans shows a startup camera picker before boot.
- Without a TTY, the picker is skipped automatically.
- The web UI is served on `http://127.0.0.1:8080/` unless you override `--port`.

## Tests

```bash
cargo test --workspace
```

## Release Artifacts

Official release builds target x86_64 on:

- Linux: `Observans-linux-x64.tar.gz`
- Windows: `Observans-windows-x64.zip`

Both release bundles contain:

- the Observans executable
- `_observans_runtime/ffmpeg/bin/...`
- a release-focused `README.md`
- `_observans_runtime/build_meta.json`

Linux also ships release-level `install.sh` and `uninstall.sh`.

## Release Builds

Release packaging entrypoints:

```bash
bash tools/build_release_linux.sh
```

```powershell
./tools/build_release_windows.ps1
```

See [BUILDING.md](BUILDING.md) for release packaging details.
