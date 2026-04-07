# Архітектура Observans v0.1.0

- Date: **2026-04-07**
- Status: **Затверджено**
- Note: **Актуалізований файл документації для поточного стану `architecture Observans v0.1.0`.**

## Загальний опис

Запуск проходить два послідовних етапи, після чого система розходиться на незалежні підсистеми:

**Етап 1 — bootstrap**

`Config::from_args_with_bootstrap()` визначає платформу, знаходить FFmpeg, збирає camera inventory і запускає TUI picker.

**Етап 2 — main()**

`main()` ініціалізує shared state і розпаровує runtime:

- `SharedLogBuffer` + tracing layer
- `SharedMetrics`
- `ClientGate`
- `SharedNetworkPolicy`
- `spawn_dashboard()`
- `spawn_system_sampler()`
- `start_capture()`
- `serve()`

Після старту `observans-web` менеджить loopback, Tailscale і за потреби LAN listeners; `observans-core::capture` паркується до першого клієнта.

## Ключова ідея

Observans побудований навколо **viewer-driven capture**:

- без клієнтів камера не захоплюється
- перший клієнт будить capture-thread
- останній клієнт зупиняє FFmpeg і звільняє пристрій

Цю поведінку реалізує `ClientGate` у [`observans-bus/src/lib.rs`](../../../../../observans-bus/src/lib.rs).

## Workspace і ролі crates

| Crate | Роль |
|---|---|
| `observans` | root binary: startup orchestration, tracing, shutdown wiring |
| `observans-core` | config, platform logic, camera inventory, probe, capture, metrics, TUI |
| `observans-bus` | broadcast bus для JPEG-кадрів і `ClientGate` |
| `observans-web` | axum router, `/stream`, `/metrics`, embedded UI |

## Підсистеми

### 1. Startup і config

[`observans-core/src/config.rs`](../../../../../observans-core/src/config.rs) виконує:

- визначення платформи
- перевірку interactive terminal
- пошук FFmpeg
- bootstrap вибору камери
- стартовий network policy через `--allow-lan`
- фінальний `clap` parse

Bootstrap picker — [`observans-core/src/bootstrap.rs`](../../../../../observans-core/src/bootstrap.rs).  
TUI picker/dashboard — [`observans-core/src/tui.rs`](../../../../../observans-core/src/tui.rs).

### 2. Network policy

[`observans-core/src/network.rs`](../../../../../observans-core/src/network.rs) централізує мережеву модель через `SharedNetworkPolicy`.

Default поведінка fail-closed:

- `127.0.0.1:<port>` — слухається завжди
- `Tailscale_IP:<port>` — підіймається best-effort, якщо адреса знайдена
- LAN listeners — не відкриваються без явного дозволу оператора

При `LAN → OFF` web layer негайно зупиняє accept на LAN listeners і обриває активні LAN `/stream` сесії через watch-based policy signal.

### 3. Inventory і probe

Два окремі етапи:

**Inventory** — знайти список камер:

- Linux: `v4l2-ctl --list-devices`, fallback на сканування `/dev/video0..63`
- Windows: `ffmpeg -list_devices true -f dshow -i dummy`, fallback на `ffmpeg -sources dshow`

**Probe** — знайти підтримувані режими конкретної камери:

- Linux: `v4l2-ctl --list-formats-ext`, fallback на `ffmpeg -f v4l2 -list_formats all`
- Windows: `ffmpeg -f dshow -list_options true -i video=<name>`

Реалізація:

- [`observans-core/src/camera_inventory.rs`](../../../../../observans-core/src/camera_inventory.rs)
- [`observans-core/src/probe.rs`](../../../../../observans-core/src/probe.rs)

### 4. Capture supervisor

Capture працює в окремому OS thread, поза async runtime:

1. Чекає на `gate.wait_for_clients()`
2. Виконує `resolve_device_candidates()`
3. Запускає probe
4. Формує пріоритетний список FFmpeg attempts
5. Читає stdout FFmpeg, парсить JPEG stream, шле кадри в broadcast channel

Якщо клієнти зникають — child-process убивається і обов'язково `wait()`-иться, щоб камера реально звільнилася. У `auto` режимі supervisor може пройти кілька кандидатів, якщо перший не дав жодного кадру.

### 5. Web layer

[`observans-web/src/lib.rs`](../../../../../observans-web/src/lib.rs) — три endpoints:

| Route | Призначення |
|---|---|
| `/` | Вбудований HTML з inline CSS/JS |
| `/metrics` | JSON snapshot метрик |
| `/stream` | MJPEG multipart stream |

Web runtime одночасно тримає кілька listeners і перевіряє peer ACL (`loopback`, `tailscale`, `private-lan`) перед кожним запитом.

### 6. Metrics, sensors і логи

[`observans-core/src/metrics.rs`](../../../../../observans-core/src/metrics.rs) збирає:

- CPU, RAM, temperature, battery
- clients, uptime
- actual / target FPS, frame age, average frame size
- queue drops, restart count, stream input, capture backend

Допоміжні модулі:

- [`observans-core/src/sensors.rs`](../../../../../observans-core/src/sensors.rs) — best-effort sensor sampling
- [`observans-core/src/logs.rs`](../../../../../observans-core/src/logs.rs) — tokenized runtime log buffer
- [`src/log_capture.rs`](../../../../../src/log_capture.rs) — tracing layer для TUI

### 7. Shutdown

Graceful shutdown через [`observans-core/src/shutdown.rs`](../../../../../observans-core/src/shutdown.rs):

- `Ctrl+C` у процесі
- `Ctrl+C`, `Q` або `Esc` у dashboard
- `Shutdown::trigger()` будить waiters
- axum завершується через `with_graceful_shutdown`

## Потоки даних

### Потік кадрів
```
camera → ffmpeg subprocess → stdout bytes → JpegStreamParser
  → FrameSender (broadcast) → /stream receivers
  → browser MJPEG source → canvas render loop
```

## Потоки даних

### Потік кадрів

Кадри рухаються від пристрою до браузера через односпрямований pipeline:

1. FFmpeg читає з камери і пише JPEG-байти в stdout
2. `JpegStreamParser` розбирає байтовий потік на окремі кадри
3. `FrameSender` розсилає кадри через broadcast channel
4. Кожен `/stream` handler отримує свою копію і відправляє клієнту
5. Браузер рендерить MJPEG через canvas render loop

### Потік керування capture

Capture supervisor реагує на зміни кількості клієнтів і стану мережевої policy:

- **Перший клієнт підключається**   — `ClientGate::add_client()` будить capture thread, виконується probe і стартує FFmpeg
- **Оператор вимикає LAN у TUI**    — `SharedNetworkPolicy::toggle_lan()` сигналізує web layer, активні LAN `/stream` сесії обриваються негайно
- **Останній клієнт від'єднується** — `ClientGate::remove_client()` зупиняє FFmpeg, камера звільняється

### Потік телеметрії

`sysinfo` і platform sensors оновлюють `SharedMetrics` раз на секунду. `app.js` у браузері опитує `GET /metrics` і оновлює DOM без перезавантаження сторінки.