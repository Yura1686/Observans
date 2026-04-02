# Running Observans

## Development

1. Install Rust and `ffmpeg`.
2. Ensure your camera is available to FFmpeg on the current platform.
3. Start the app:

```bash
cargo run
```

Open:

```text
http://127.0.0.1:8080/
```

## Runtime behavior

- Interactive terminal: startup camera picker is shown unless `--device` or `--no-camera-select` is passed.
- Non-interactive launch: picker is skipped and Observans boots directly.
- `--device auto`: Observans resolves the first discovered camera and falls back to the platform default if discovery returns nothing.

## Cross-platform device notes

- Linux capture format: `v4l2`
- Windows capture format: `dshow`
- macOS capture format: `avfoundation`

## FFmpeg override

Set `OBSERVANS_FFMPEG` to point to a specific FFmpeg binary:

```bash
OBSERVANS_FFMPEG=/path/to/ffmpeg cargo run
```

Runtime lookup order is:

1. `OBSERVANS_FFMPEG`
2. bundled `_observans_runtime/ffmpeg/bin/ffmpeg(.exe)` next to the executable
3. `ffmpeg` from `PATH`

## Release usage

Linux release users can:

- run `install.sh`
- or unpack `Observans-linux-x64.tar.gz` and start `./observans`

Windows release users can:

- unzip `Observans-windows-x64.zip`
- run `observans.exe`

