# Observans Release

This bundle is ready to run after extraction.

## Windows

1. Unzip `Observans-windows-x64.zip`
2. Open the extracted folder
3. Run `observans.exe`
4. Open `http://127.0.0.1:8080/`

## Linux

Option 1:

```bash
bash install.sh
```

Option 2:

```bash
tar -xzf Observans-linux-x64.tar.gz
cd Observans-linux-x64
./observans
```

Then open:

```text
http://127.0.0.1:8080/
```

## Notes

- Bundled FFmpeg is included in `_observans_runtime/ffmpeg/bin`
- `OBSERVANS_FFMPEG` overrides bundled FFmpeg lookup
- Startup camera picker appears only in interactive terminals
- `observans --help` shows all runtime flags

