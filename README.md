# Observans

Observans is a local-first camera streaming workspace for Linux and Windows.
It exposes a browser-based monitoring panel, a startup terminal camera picker,
and a lightweight MJPEG pipeline designed for simple LAN viewing and operator
control.

## What Observans Provides

- Startup camera selection in an interactive terminal with an ASCII/ANSI TUI.
- A local web panel with live stream preview, browser-side recording, fullscreen
  viewing, and host telemetry.
- MJPEG streaming over HTTP for low-friction local access.
- Runtime telemetry for CPU, RAM, temperature, battery, viewer count, restart
  count, frame age, and frame size.
- A demand-driven capture model: the camera pipeline wakes when the first viewer
  connects and stops when the last viewer leaves.

## Supported Platforms

Observans currently supports:

- Linux
- Windows

Unsupported operating systems fail fast during startup with a clear platform
error.

## Workspace Layout

The repository is a Cargo workspace with four packages:

| Path              | Responsibility                                                                           |
| ----------------- | ---------------------------------------------------------------------------------------- |
| `src/`            | Root application bootstrap, tracing setup, runtime wiring                                |
| `observans-core/` | CLI config, platform detection, camera inventory, TUI, metrics, sensors, capture runtime |
| `observans-bus/`  | Shared frame bus and client gate primitives                                              |
| `observans-web/`  | Axum web server, MJPEG endpoint, metrics endpoint, embedded UI assets                    |

## Quick Start

### Prerequisites

- Rust toolchain with `cargo`
- `ffmpeg` available on `PATH`, unless you provide `OBSERVANS_FFMPEG`
- Linux only: `v4l2-ctl` is optional, but improves device discovery and probing

### Start In Development

```bash
cargo run
```

By default Observans serves the web UI on:

```text
http://127.0.0.1:8080/
```

The browser is not opened automatically. Observans prints its listening address
to the console and waits for viewers to connect.

## Runtime Model

- In an interactive terminal, Observans shows a startup camera picker unless
  you pass `--device` explicitly or use `--no-camera-select`.
- In a non-interactive launch, the picker is skipped automatically.
- `--device auto` resolves the first available camera for the current platform.
- The capture backend is FFmpeg-based: `v4l2` on Linux and `dshow` on Windows.
- The capture process is idle until a viewer opens the stream, which helps keep
  the camera released when the system is unattended.

## Web Endpoints

| Endpoint | Purpose |
| --- | --- |
| `/` | Main browser UI |
| `/stream` | MJPEG live stream |
| `/metrics` | JSON metrics snapshot for the current runtime |

## CLI

Current runtime options:

```text
observans [OPTIONS]

--port <PORT>                  default: 8080
--device <DEVICE>              default: auto
--width <WIDTH>                default: 1280
--height <HEIGHT>              default: 720
--fps <FPS>                    default: 30
--input-format <INPUT_FORMAT>  auto | mjpeg | yuyv422 | uyvy422 | nv12 | h264
--no-camera-select
```

Example:

```bash
cargo run -- --device auto --port 8080 --width 1280 --height 720 --fps 30
```

## FFmpeg Resolution Order

Observans resolves FFmpeg in the following order:

1. `OBSERVANS_FFMPEG`
2. Bundled runtime FFmpeg next to the executable in release bundles
3. `ffmpeg` from `PATH`

Example override:

```bash
OBSERVANS_FFMPEG=/path/to/ffmpeg cargo run
```

## Web Panel Features

The browser UI currently includes:

- Live MJPEG preview
- Fullscreen viewing
- Local browser-side recording and save-to-disk
- Stream health indicators
- Host telemetry
- Battery and temperature reporting when the current platform exposes them

Temperature and battery collection are best-effort. Desktop systems without a
battery, firmware without readable thermal sensors, or restrictive Windows
sensor exposure may report unavailable values.

## Release Artifacts

Official release bundles target `x86_64` only:

- Linux: `Observans-linux-x64.tar.gz`
- Windows: `Observans-windows-x64.zip`

Bundle layout:

- Linux archive entrypoint: `Observans.sh`
- Linux runtime binary: `_observans_runtime/bin/observans`
- Windows entrypoint: `observans.exe`
- Bundled FFmpeg: `_observans_runtime/ffmpeg/bin/...`
- Embedded release guide: `README.md`
- Build metadata: `_observans_runtime/build_meta.json`

GitHub Releases are published as one rolling pre-release refreshed from the
latest successful build of `main`.

## Development

Run the full workspace test suite:

```bash
cargo test --workspace
```

Useful docs:

- [BUILDING.md](BUILDING.md) for release packaging and artifact generation
- [RELEASE_README.md](RELEASE_README.md) for the end-user guide embedded into
  release bundles

## Release Packaging

Release bundles are built with:

```bash
bash tools/build_release_linux.sh
```

```powershell
./tools/build_release_windows.ps1
```

Linux packaging also emits local helper outputs in `dist/`:

- `install.sh`
- `uninstall.sh`

These helper scripts are local packaging outputs and are not part of the rolling
GitHub Release assets.
