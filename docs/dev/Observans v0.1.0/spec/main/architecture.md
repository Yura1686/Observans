# Архітектура Observans

- Date:   **2026-04-07**
- Status: **Затверджено**
- Note:   **Актуалізований файл документації для поточного стану `architecture Observans v0.1.0`.** 

## Загальна картина

```text
    terminal / CLI
    |
    v
    Config::from_args_with_bootstrap()
    |
    +--> camera inventory + TUI picker
    |
    v
    main()
    |
    +--> SharedLogBuffer + tracing layer
    +--> SharedMetrics
    +--> ClientGate
    +--> spawn_dashboard()
    +--> spawn_system_sampler()
    +--> start_capture()        
    +--> serve()             
```

Після старту система розходиться на три незалежні напрями:

- `observans-web` приймає HTTP-запити
- `observans-core::metrics` раз на секунду оновлює system telemetry
- `observans-core::capture` чекає на підключення глядача і тільки тоді запускає FFmpeg

## Ключова ідея поточної реалізації

Observans тепер побудований навколо **viewer-driven capture**:

- без клієнтів камера не захоплюється
- перший клієнт будить capture-thread
- останній клієнт зупиняє FFmpeg і звільняє пристрій

Цю поведінку реалізує [`observans-bus/src/lib.rs`](../../../../../observans-bus/src/lib.rs) через `ClientGate`.

## Workspace і ролі crates

| Crate | Роль |
| --- | --- |
| `observans` | root binary: startup orchestration, tracing, shutdown wiring |
| `observans-core` | config, platform logic, camera inventory, probe, capture, metrics, TUI |
| `observans-bus` | broadcast bus для JPEG-кадрів і `ClientGate` |
| `observans-web` | axum router, `/stream`, `/metrics`, embedded UI |

## Головні підсистеми

### 1. Startup і config

Startup збирається навколо [`observans-core/src/config.rs`](../../../../../observans-core/src/config.rs):

- визначення платформи
- перевірка interactive terminal
- пошук FFmpeg
- bootstrap вибору камери
- фінальний `clap` parse

Bootstrap picker мешкає у [`observans-core/src/bootstrap.rs`](../../../../../observans-core/src/bootstrap.rs), а TUI picker/dashboard - у [`observans-core/src/tui.rs`](../../../../../observans-core/src/tui.rs).

### 2. Inventory + probe

Observans розділяє два різні етапи:

- **inventory**: знайти список камер
- **probe**: знайти підтримувані режими конкретної камери

Inventory:

- Linux: `v4l2-ctl --list-devices`, fallback на сканування `/dev/video0..63`
- Windows: `ffmpeg -list_devices true -f dshow -i dummy`, fallback на `ffmpeg -sources dshow`

Probe:

- Linux: `v4l2-ctl --list-formats-ext`, fallback на `ffmpeg -f v4l2 -list_formats all`
- Windows: `ffmpeg -f dshow -list_options true -i video=<name>`

Ця логіка зосереджена у:

- [`observans-core/src/camera_inventory.rs`](../../../../../observans-core/src/camera_inventory.rs)
- [`observans-core/src/probe.rs`](../../../../../observans-core/src/probe.rs)

### 3. Capture supervisor

Capture не живе в async runtime. Він працює в окремому OS thread:

- чекає на `gate.wait_for_clients()`
- робить `resolve_device()`
- запускає probe
- формує пріоритетний список FFmpeg attempts
- читає stdout FFmpeg
- парсить JPEG stream
- шле кадри в broadcast channel

Якщо клієнти зникають, child-process убивається й обов'язково `wait()`-иться, щоб камера реально звільнилася.

### 4. Web layer

[`observans-web/src/lib.rs`](../../../../../observans-web/src/lib.rs) експонує три основні endpoints:

| Route | Призначення |
| --- | --- |
| `/` | Вбудований HTML з inline CSS/JS |
| `/metrics` | JSON snapshot метрик |
| `/stream` | MJPEG multipart stream |

`AppState` тримає:

- `tx: FrameSender`
- `metrics: SharedMetrics`
- `gate: Arc<ClientGate>`
- `config: Config`

### 5. Metrics, sensors і logs

Metrics збираються в [`observans-core/src/metrics.rs`](../../../../../observans-core/src/metrics.rs) і містять:

- CPU / RAM
- temperature / battery
- clients / uptime
- actual FPS / target FPS
- frame age / average frame size
- queue drops / restart count
- stream input / capture backend

Допоміжні підсистеми:

- [`observans-core/src/sensors.rs`](../../../../../observans-core/src/sensors.rs): best-effort sensor sampling
- [`observans-core/src/logs.rs`](../../../../../observans-core/src/logs.rs): tokenized runtime log buffer
- [`src/log_capture.rs`](../../../../../src/log_capture.rs): tracing layer, що складає log entries для TUI

### 6. Shutdown

Graceful shutdown тепер централізований:

- `Ctrl+C` у процесі
- `Ctrl+C`, `Q` або `Esc` у dashboard
- `Shutdown::trigger()` будить waiters
- axum завершується через `with_graceful_shutdown`

Це реалізовано у [`observans-core/src/shutdown.rs`](../../../../../observans-core/src/shutdown.rs).

## Потоки даних

### Потік кадрів

```text
camera
  -> ffmpeg subprocess
  -> stdout bytes
  -> JpegStreamParser
  -> FrameSender (broadcast)
  -> /stream receivers
  -> browser <img> MJPEG source
  -> <canvas> render loop
```

### Потік керування capture

```text
browser opens /stream
  -> ClientGuard::new()
  -> AppState::client_connected()
  -> ClientGate::add_client()
  -> capture thread wakes up

browser closes /stream
  -> ClientGuard::drop()
  -> AppState::client_disconnected()
  -> ClientGate::remove_client()
  -> capture loop kills ffmpeg when count reaches zero
```

### Потік телеметрії

```text
sysinfo + platform sensor readers
  -> SharedMetrics
  -> GET /metrics
  -> app.js poll each second
  -> DOM updates
```

## Паралелізм

| Частина | Тип виконання |
| --- | --- |
| Axum server | async tokio runtime |
| `/stream` handler на клієнта | async task |
| system sampler | async task |
| dashboard | окремий OS thread |
| capture supervisor | окремий OS thread |
| stderr collector | окремий OS thread |
| FFmpeg stdout reader | окремий OS thread |
