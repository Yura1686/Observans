# Startup Flow

- Date:   **2026-04-07**
- Status: **Затверджено**
- Note:   **Актуалізований файл документації для поточного стану  `startup-flow Observans v0.1.0`.** 

## Від процесу до першого кадру

```text
    cargo run / observans / Observans.sh / observans.exe
    |
    v
    main()
    |
    +--> terminal_is_interactive()
    +--> Config::from_args_with_bootstrap()
    +--> init_tracing()
    +--> SharedMetrics + SharedLogBuffer + ClientGate + Shutdown
    +--> spawn_dashboard()            
    +--> spawn_system_sampler()
    +--> start_capture()              
    +--> serve()
```

**Після старту процесу перший кадр ще не генерується**. Capture-thread уже існує, але він спить, поки не з'явиться viewer.

## Крок 1. Визначення режиму термінала

[`src/main.rs`](../../../../../src/main.rs) починає з:

- `terminal_is_interactive()`

Це рішення впливає на дві речі:

- чи показувати startup camera picker
- чи рендерити console logs напряму, чи віддати все TUI dashboard

## Крок 2. `Config::from_args_with_bootstrap(...)`

Під капотом це послідовність із [`observans-core/src/config.rs`](../../../../../observans-core/src/config.rs):

1. зібрати raw args
2. перевірити платформу
3. визначити interactive mode
4. знайти FFmpeg binary
5. викликати `patch_args_for_camera_selection(...)`
6. розпарсити фінальні args через `clap`

## Крок 3. Camera picker bootstrap

Picker вмикається тільки якщо одночасно виконуються умови:

- у CLI немає `--device`
- немає `--no-camera-select`
- термінал interactive

Тоді flow такий:

```text
    patch_args_for_camera_selection()
    |
    +--> enumerate_cameras()
    |
    +--> choose_camera()
    |
    +--> if selected:
            append --device <chosen>
```

Можливі гілки:

- знайдені камери -> користувач обирає конкретну або `auto`
- камер немає -> у args додається `--device auto`
- inventory/picker помиляється -> startup триває без аварії
- `Ctrl+C` усередині picker -> startup переривається помилкою

## Крок 4. Ініціалізація runtime state

Після конфігу `main()` створює:

- `SharedLogBuffer`
- tracing registry з `UiLogLayer`
- `Shutdown`
- `SharedMetrics`
- broadcast bus через `create_bus(4)`
- `ClientGate`

Одразу після цього в log buffer додається запис про resolved device.

## Крок 5. Signal listener

У `main()` запускається async task, яка слухає `tokio::signal::ctrl_c()`.

На `Ctrl+C` відбувається:

1. лог у buffer
2. `shutdown.trigger()`
3. graceful shutdown для server side задач

## Крок 6. Dashboard

Якщо terminal interactive, стартує [`spawn_dashboard()`](../../../../../observans-core/src/tui.rs).

Dashboard - це не startup picker, а окремий live-екран, який далі показує:

- доступні URLs
- конфіг і поточну камеру
- live metrics
- event feed
- hotkeys

Hotkeys:

- `Ctrl+C` -> graceful shutdown
- `Q` або `Esc` -> shutdown через dashboard

## Крок 7. System sampler

`spawn_system_sampler(metrics.clone())` запускає async loop із кроком 1 секунда:

- refresh CPU
- refresh memory
- sample temperature
- sample battery
- update `SharedMetrics`

Це ще не означає, що стрім уже живий. Телеметрія може оновлюватися до появи першого кадру.

## Крок 8. Capture supervisor стартує, але паркується

`start_capture(config, tx, metrics, gate)` одразу створює thread, але перший рядок реальної роботи там:

```text
gate.wait_for_clients()
```

Тобто pipeline стоїть у режимі очікування.

## Крок 9. Web server починає слухати порт

`serve(bind_addr, state, shutdown)`:

- будує router
- `TcpListener::bind(0.0.0.0:port)`
- логгує локальний URL
- чекає HTTP-запити

На цьому етапі:

- `/` уже доступний
- `/metrics` уже доступний
- `/stream` теж доступний
- камера ще може бути не відкрита

## Крок 10. Перший viewer будить capture

Коли браузер відкриває `/stream`:

1. `mjpeg_handler()` створює receiver через `state.tx.subscribe()`
2. `ClientGuard::new()` викликає `state.client_connected()`
3. `AppState::client_connected()` робить `gate.add_client()`
4. capture-thread прокидається

Це і є точка реального переходу від idle до active capture.

## Крок 11. Capture session

Після wake-up:

1. `resolve_device()`
2. `probe_camera()`
3. `metrics.set_stream_input(...)`
4. `build_capture_attempts(...)`
5. запуск першої FFmpeg attempt

Перший кадр з'являється тільки після того, як:

- FFmpeg стартував
- stdout почав віддавати JPEG bytes
- `JpegStreamParser` витягнув повний JPEG
- frame був відправлений у broadcast channel

## Крок 12. Браузер отримує перший кадр

У [`observans-web/assets/app.js`](../../../../../observans-web/assets/app.js) логіка така:

1. `connectStream()` встановлює `img#stream-source.src = "/stream?..."`
2. коли браузер отримує JPEG, спрацьовує `streamSource.onload`
3. UI переходить у `is-live`
4. запускається canvas render loop
5. `tick()` починає показувати фактичні stream metrics

## Якщо viewer пішов

У stream handler є `ClientGuard::drop()`. Коли клієнт зникає:

- `gate.remove_client()`
- metrics.clients оновлюється
- capture loop перевіряє gate
- FFmpeg child убивається
- `wait()` завершує процес
- камера звільняється
- capture-thread знову йде в `wait_for_clients()`

## Shutdown Flow

Поточний shutdown уже не описується як "процес просто вмирає":

```text
    Ctrl+C або Q/Esc у dashboard
    |
    v
    Shutdown::trigger()
    |
    v
    axum graceful shutdown
    |
    v
    main() виходить із serve()
    |
    v
    shutdown.trigger() повторно для надійності
    |
    v
    dashboard thread join()
```

Capture-thread не має окремого `Shutdown` wait, але завершується разом із процесом після завершення runtime.
