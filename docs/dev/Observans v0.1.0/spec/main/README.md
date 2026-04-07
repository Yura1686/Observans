# Observans v0.1.0
  
- Date:   **2026-04-07**
- Status: **Затверджено**
- Note:   **Актуалізований комплект документації для поточного стану `workspace Observans Observans v0.1.0`.** 

## Що це 

**Observans** - локальний Rust-застосунок для стрімінгу відео з вебкамери у браузер через MJPEG. Поточна реалізація вже не тримає камеру постійно відкритою: capture-пайплайн паркується без глядачів і запускає FFmpeg тільки тоді, коли перший клієнт відкриває `/stream`.

Основні можливості поточного стану:

- startup camera picker у терміналі
- live TUI dashboard з телеметрією та логами
- on-demand capture через `ClientGate`
- MJPEG streaming через `/stream`
- browser UI з fullscreen, локальним записом і live-метриками
- Linux (`v4l2`) і Windows (`dshow`) backend через FFmpeg
- probe-логіка для підбору кращого формату, роздільності й FPS
- rolling release pipeline для Linux і Windows

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

| Директорія | Призначення |
| --- | --- |
| [../core/README.md](../core/README.md) | Навігація по матеріалах `main`, що стосуються `observans-core` |
| [../web/README.md](../web/README.md) | Навігація по матеріалах `main`, що стосуються `observans-web` |
| [../bus/README.md](../bus/README.md) | Навігація по матеріалах `main`, що стосуються `observans-bus` |

## Поточна runtime-модель

1. `main()` читає CLI й виконує bootstrap вибору камери.
2. Якщо термінал інтерактивний, запускається TUI dashboard.
3. Web server стартує одразу, але capture-thread чекає на глядача.
4. Перший клієнт на `/stream` збільшує `ClientGate` і будить capture.
5. Capture робить probe, будує набір FFmpeg-спроб і починає передавати JPEG-кадри в broadcast bus.
6. Коли останній клієнт від'єднується, FFmpeg process вбивається, камера звільняється, pipeline знову паркується.


