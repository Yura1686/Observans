# Observans v0.1.0
  
- Date:   **2026-04-07**
- Status: **Затверджено**
- Note:   **Актуалізований комплект документації для поточного стану `workspace Observans Observans v0.1.0`.** 

## Що це 

**Observans** - локальний Rust-застосунок для стрімінгу відео з вебкамери у браузер через MJPEG. 

Основні можливості Observans v0.1.0:

- Вибір камери при запуску
- Live TUI dashboard — телеметрія, логи, перемикання LAN у runtime
- On-demand capture через `ClientGate`
- MJPEG-стрімінг на `/stream`
- Browser UI — fullscreen, локальний запис, live-metrics
- Linux `v4l2` та Windows `dshow` бекенди через FFmpeg
- Probe-логіка для автоматичного вибору формату, роздільності та FPS
- Secure-by-default мережева модель — `127.0.0.1` завжди, Tailscale best-effort, LAN вимкнений за замовчуванням
- Rolling release pipeline для Linux та Windows

## Навігація

| Файл                                       | Що описує                                               |
|--------------------------------------------|-------------------------------------------------------- |
| [architecture.md](architecture.md)         | Актуальна архітектура, підсистеми, потоки даних         |
| [crates.md](crates.md)                     | Розбір workspace, crates і головних файлів              |
| [startup-flow.md](startup-flow.md)         | Шлях від запуску процесу до першого кадру               |
| [capture-pipeline.md](capture-pipeline.md) | Capture supervisor, probe, FFmpeg attempts, JPEG parser |
| [web-ui.md](web-ui.md)                     | Поточний embedded frontend: HTML, CSS, JS               |
| [release.md](release.md)                   | Release manifest, packagers, GitHub workflows           |

## Тематичні директорії spec

| Директорія                             | Призначення                                                    |
| -------------------------------------- | -------------------------------------------------------------- |
| [../core/README.md](../core/README.md) | Навігація по матеріалах `main`, що стосуються `observans-core` |
| [../web/README.md](../web/README.md)   | Навігація по матеріалах `main`, що стосуються `observans-web`  |
| [../bus/README.md](../bus/README.md)   | Навігація по матеріалах `main`, що стосуються `observans-bus`  |

## Поточна runtime-модель

1. `main()` читає CLI й виконує bootstrap вибору камери.
2. Якщо термінал інтерактивний, запускається TUI dashboard.
3. Web server стартує одразу, але за default слухає тільки `127.0.0.1` і Tailscale IP, якщо він доступний.
4. Перший клієнт на `/stream` збільшує `ClientGate` і будить capture.
5. Capture робить probe, будує набір FFmpeg-спроб і починає передавати JPEG-кадри в broadcast bus.
6. Коли останній клієнт від'єднується, FFmpeg process вбивається, камера звільняється, pipeline знову паркується.
7. LAN доступ можна ввімкнути або вимкнути під час роботи через TUI; при `LAN OFF` активні LAN stream-сесії обриваються одразу.

