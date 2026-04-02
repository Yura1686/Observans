# Observans Release

This bundle is ready to run after extraction on Linux or Windows.

GitHub keeps this archive in a rolling pre-release that is refreshed from the latest successful `main` build.

## Windows

1. Unzip `Observans-windows-x64.zip`
2. Open the extracted folder
3. Run `observans.exe`
4. Watch the console for the local URL, for example `http://127.0.0.1:8080/`
5. Open that URL manually in your browser

## Linux

1. Unpack `Observans-linux-x64.tar.gz`
2. Open the extracted folder
3. Run `Observans.sh` for the launcher path
4. Or run `./observans` directly from a terminal
5. Watch the console for the local URL, for example `http://127.0.0.1:8080/`
6. Open that URL manually in your browser

The Linux bundle also includes the raw `observans` binary next to the launcher.

## Notes

- Bundled FFmpeg is included in `_observans_runtime/ffmpeg/bin`
- `OBSERVANS_FFMPEG` overrides bundled FFmpeg lookup
- Startup camera picker appears only in interactive terminals
- Observans does not open your browser automatically
- `observans --help` shows all runtime flags
