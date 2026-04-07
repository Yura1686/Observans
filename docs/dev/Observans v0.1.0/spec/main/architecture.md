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
    +--> SharedNetworkPolicy
    +--> spawn_dashboard()
    +--> spawn_system_sampler()
    +--> start_capture()        
    +--> serve()             
```

Після старту система розходиться на кілька незалежних напрямів:

- `observans-web` менеджить loopback, tailscale і за потреби LAN listeners
- `observans-core::metrics` раз на секунду оновлює system telemetry
- `observans-core::capture` чекає на підключення глядача і тільки тоді запускає FFmpeg
- `observans-core::network` тримає shared network policy для TUI і web runtime

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
- стартовий network policy через `--allow-lan`
- фінальний `clap` parse

Bootstrap picker мешкає у [`observans-core/src/bootstrap.rs`](../../../../../observans-core/src/bootstrap.rs), а TUI picker/dashboard - у [`observans-core/src/tui.rs`](../../../../../observans-core/src/tui.rs).

### 1.5. Network policy

[`observans-core/src/network.rs`](../../../../../observans-core/src/network.rs) централізує мережеву модель:

- `SharedNetworkPolicy` з runtime прапором `lan_enabled`
- discovery Tailscale IPv4 через `tailscale ip -4`
- discovery private IPv4 адрес хоста для LAN listener-ів
- побудову бажаного набору listener-ів
- peer ACL класифікацію для `loopback`, `tailscale`, `private-lan`

Default поведінка fail-closed:

- `127.0.0.1:<port>` слухається завжди
- `Tailscale_IP:<port>` підіймається best-effort, якщо адреса знайдена
- private LAN listeners не відкриваються, поки оператор явно не ввімкне LAN

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
- робить `resolve_device_candidates()`
- запускає probe
- формує пріоритетний список FFmpeg attempts
- читає stdout FFmpeg
- парсить JPEG stream
- шле кадри в broadcast channel

Якщо клієнти зникають, child-process убивається й обов'язково `wait()`-иться, щоб камера реально звільнилася.

У `auto` режимі supervisor тепер може пройти кілька кандидатів камери, якщо перший Windows/Linux device id не зміг дати жодного кадру на старті.

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
- `network: SharedNetworkPolicy`

`serve()` тепер не працює як один `TcpListener`. Поточний web runtime:

- завжди тримає loopback listener
- best-effort підіймає tailscale listener
- додає або прибирає LAN listeners під час роботи, коли policy змінюється
- додає `ListenerKind` у request context
- перевіряє peer ACL перед `/`, `/metrics` і `/stream`

При `LAN -> OFF` web layer:

- зупиняє accept на LAN listeners
- одразу обриває активні LAN `/stream` сесії через watch-based policy signal
- не чіпає loopback і tailscale viewers

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

operator presses L in TUI
  -> SharedNetworkPolicy::toggle_lan()
  -> web listener manager reconciles listeners
  -> active LAN /stream sessions terminate immediately if policy became OFF

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
| listener manager | async tokio runtime |
| `/stream` handler на клієнта | async task |
| system sampler | async task |
| dashboard | окремий OS thread |
| capture supervisor | окремий OS thread |
| stderr collector | окремий OS thread |
| FFmpeg stdout reader | окремий OS thread |
