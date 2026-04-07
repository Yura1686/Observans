# Крeйти та файли

- Date:   **2026-04-07**
- Status: **Затверджено**
- Note:   **Актуалізований файл документації для поточного стану `crates Observans v0.1.0`.** 

## Workspace

```text
Cargo workspace
├── observans         # root binary
├── observans-core    # основна бізнес-логіка
├── observans-bus     # shared transport / gating
└── observans-web     # HTTP server + UI
```

## `observans` (root)

### `src/main.rs`

Поточна точка входу:

1. визначає, чи термінал інтерактивний
2. читає `Config::from_args_with_bootstrap(...)`
3. створює `SharedLogBuffer`
4. ініціалізує tracing і вбудований UI log layer
5. створює `Shutdown`, `SharedMetrics`, frame bus і `ClientGate`
6. запускає signal listener для `Ctrl+C`
7. за потреби запускає dashboard
8. запускає system sampler і capture supervisor
9. підіймає web server

### `src/log_capture.rs`

Міст між `tracing` і TUI log buffer:

- читає `Event`
- витягує `message` та поля
- класифікує лог у термінах `LogLevel`
- відправляє запис у `SharedLogBuffer`

## `observans-bus`

### `observans-bus/src/lib.rs`

Містить два окремі контракти:

- `broadcast::channel` для JPEG frames
- `ClientGate` для координації життєвого циклу capture

#### Що робить `ClientGate`

- збільшує лічильник при відкритті `/stream`
- зменшує при розриві клієнта
- будить capture-thread через `Condvar`
- дозволяє швидко перевірити, чи ще є глядачі

## `observans-core`

### `observans-core/src/lib.rs`

Публічний фасад crate. Реекспортує:

- config
- inventory
- capture
- metrics
- probe
- logs
- shutdown
- TUI helpers

### `observans-core/src/config.rs`

`clap`-конфіг runtime:

| Аргумент | Default |
| --- | --- |
| `--port` | `8080` |
| `--device` | `auto` |
| `--width` | `1280` |
| `--height` | `720` |
| `--fps` | `30` |
| `--input-format` | `auto` |
| `--no-camera-select` | `false` |

Корисні helper methods:

- `platform_name()`
- `capture_format()`
- `platform_default_device()`
- `bind_addr()`
- `capture_backend_label()`

### `observans-core/src/bootstrap.rs`

Патчить raw CLI args для startup camera picker.

Поточна логіка:

- якщо вже є `--device`, нічого не змінює
- якщо є `--no-camera-select`, нічого не змінює
- якщо terminal не interactive, нічого не змінює
- якщо камер немає, додає `--device auto`
- якщо picker повернув конкретний device, додає його в args
- якщо picker зірвався через `Ctrl+C`, повертає помилку нагору
- інші помилки не валять startup

### `observans-core/src/platform.rs`

Мінімальний platform gate:

- підтримуються тільки `linux` і `windows`
- мапить platform -> capture backend (`v4l2` / `dshow`)
- дає platform default device

### `observans-core/src/runtime.rs`

Пошук FFmpeg у такому порядку:

1. `OBSERVANS_FFMPEG`
2. bundled runtime path поряд з executable
3. `ffmpeg` або `ffmpeg.exe` з `PATH`

### `observans-core/src/camera_inventory.rs`

Відповідає саме за **список камер**, а не за їхні режими.

Linux:

- читає `v4l2-ctl --list-devices`
- якщо треба, сканує `/dev/video0..63`
- відкидає non-capture вузли через sysfs name heuristics

Windows:

- читає DirectShow inventory із FFmpeg
- вміє підхопити `Alternative name`
- fallback на `ffmpeg -sources dshow`

### `observans-core/src/probe.rs`

Відповідає за **режими захоплення** конкретної камери.

Основні типи:

- `CameraMode`
- `ProbeResult`
- `ResolvedCaptureParams`

Що важливо:

- compressed formats (`mjpeg`, `h264`) мають високий пріоритет
- best-mode selection враховує compression, resolution, fps
- якщо користувач явно задав `--input-format`, probe не нав'язує свій формат

### `observans-core/src/capture.rs`

Найскладніший модуль runtime.

Основні обов'язки:

- supervisor loop
- on-demand parking / wake-up
- device resolution
- probe + capture attempt selection
- побудова FFmpeg args
- читання stdout і stderr
- JPEG parsing
- restart/backoff

Ключові особливості поточної реалізації:

- capture живе в окремому thread
- stdout читається ще одним thread через `mpsc`
- при відсутності клієнтів child-process убивається
- після kill викликається `wait()` для реального release камери
- Windows не отримує `-fflags nobuffer` / `-flags low_delay`

### `observans-core/src/metrics.rs`

Тримає `MetricsSnapshot` у `RwLock` та frame stats у `Mutex`.

Поля snapshot зараз включають:

- clock/date
- CPU / RAM
- temperature / battery / battery status
- hostname / platform / capture backend
- clients / uptime
- resolution / actual FPS / target FPS
- stream pipeline / stream input
- frame age / queue drops / avg frame KB / restarts

### `observans-core/src/sensors.rs`

Best-effort platform sensors:

- температура через `sysinfo::Components`, а далі fallback на sysfs / platform readers
- батарея з кешем і refresh interval

### `observans-core/src/logs.rs`

Кільцевий буфер логів для TUI:

- `SharedLogBuffer`
- `LogEntry`
- `LogLevel`
- агреговані counts для warnings/errors

Тут же живе token-стиль:

- `[....]` info
- `[++++]` ok
- `[~~~~]` wait
- `[!!!!]` warn
- `[XXXX]` error

### `observans-core/src/shutdown.rs`

Простий shared shutdown primitive на `AtomicBool + Notify`.

### `observans-core/src/tui.rs`

Тут зібрано дві різні terminal-ролі:

- startup picker
- live dashboard

Dashboard показує:

- endpoints
- telemetry snapshot
- event feed із log buffer
- hotkeys для graceful exit

## `observans-web`

### `observans-web/src/lib.rs`

HTTP composition:

- `AppState`
- `app()`
- `serve()`
- `root()`
- `metrics()`

### `observans-web/src/stream.rs`

MJPEG endpoint.

Що тут важливо:

- кожен клієнт отримує свій `broadcast` receiver
- `ClientGuard` синхронізує connect/disconnect із `ClientGate`
- keepalive comment раз на секунду дає змогу швидше виявити мертвий TCP client
- `RecvError::Lagged` збільшує `queue_drops`

### `observans-web/src/ui.rs`

Робить inline assembly UI:

- `index.html`
- `styles.css`
- `app.js`

### `observans-web/assets/index.html`

Поточний layout:

- великий stream card із canvas
- telemetry card із progress bars
- summary card з battery / stream / recording controls
- info card з clock, platform, uptime, input, pipeline

### `observans-web/assets/styles.css`

Twilight visual system:

- градієнтне небо
- panel glass effect
- responsive two-column layout
- stream placeholder, pills, bars, recording states

### `observans-web/assets/app.js`

Уся browser-side логіка:

- MJPEG connect / reconnect
- hidden `<img>` + `<canvas>` render loop
- fullscreen
- `fetch("/metrics")` раз на секунду
- browser-side recording через `MediaRecorder`
- tab visibility handling

## Тести

Важливі тести, які вже є в repo:

- `observans-web/tests/http.rs` - HTTP contracts для `/`, `/metrics`, `/stream`
- `tests/release_contracts.rs` - manifest, installer block, workflow contracts
- unit tests у `bootstrap.rs`, `capture.rs`, `platform.rs`, `logs.rs`, `metrics.rs`, `tui.rs`
