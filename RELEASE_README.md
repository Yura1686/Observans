# Observans

This package is a ready-to-run Observans release bundle for Linux or Windows.

Observans provides a local camera streaming workflow with:

- a startup terminal camera picker when launched interactively
- a browser-based control panel
- MJPEG live streaming
- browser-side recording and fullscreen viewing
- host and stream telemetry

## Quick Start

### Windows

1. Extract `Observans-windows-x64.zip`.
2. Open the extracted folder.
3. Run `observans.exe`.
4. Watch the console for the local URL, typically `http://127.0.0.1:8080/`.
5. Open that URL in your browser.

### Linux

1. Extract `Observans-linux-x64.tar.gz`.
2. Open the extracted folder.
3. Run `Observans.sh`.
4. Watch the console for the local URL, typically `http://127.0.0.1:8080/`.
5. Open that URL in your browser.

## Runtime Notes

- Observans does not open the browser automatically.
- The startup camera picker appears only in an interactive terminal session.
- If launched without a TTY, Observans starts directly with the configured or
  auto-resolved device.
- Capture starts when a viewer connects and stops again when all viewers leave.

## Useful Flags

```text
--port <PORT>
--device <DEVICE>
--width <WIDTH>
--height <HEIGHT>
--fps <FPS>
--input-format <INPUT_FORMAT>
--no-camera-select
```

Use `Observans.sh --help` on Linux or `observans.exe --help` on Windows for the
full CLI reference.

## FFmpeg

This bundle includes FFmpeg under:

```text
_observans_runtime/ffmpeg/bin
```

Observans uses bundled FFmpeg automatically when available. You can override it
with `OBSERVANS_FFMPEG` if you need to point to a different binary.

## Release Channel

GitHub publishes these archives through a rolling pre-release refreshed from the
latest successful build of the `main` branch.
