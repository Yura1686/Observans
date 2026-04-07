# Capture Pipeline

- Date:   **2026-04-07**
- Status: **Затверджено**
- Note:   **Актуалізований файл документації для поточного стану `cupture-pipeline Observans v0.1.0`.** 

## Огляд

```text
viewer connects
    v
ClientGate wakes capture thread
    v
resolve_device()
    v
probe_camera()
    v
build_capture_attempts()
    v
spawn ffmpeg
    v
stdout bytes------> JpegStreamParser -> FrameSender
    v
/stream handlers -> browser
```

## Поточний стан

Життєвий цикл:

- `park` без клієнтів
- `run` при наявності хоча б одного клієнта
- `stop` і release camera після відключення останнього клієнта

Це реалізує [`observans-core/src/capture.rs`](../../../../../observans-core/src/capture.rs) разом з [`observans-bus/src/lib.rs`](../../../../../observans-bus/src/lib.rs).

## 1. Device resolution

`resolve_device_candidates(config)` працює так:

- якщо `config.device != "auto"` -> використовує явний device
- інакше бере впорядкований список device candidates із inventory
- якщо inventory нічого не повернув -> бере `platform_default_device()`

Поточний fallback:

- Linux----> `/dev/video0`
- Windows--> `video=Integrated Camera`

Це означає, що Windows більше не валиться одразу при `auto`, якщо inventory не спрацював.

### Windows auto-selection

На Windows inventory тепер віддає пріоритет **дружній назві камери** (`video=Integrated Camera` тощо), а `Alternative name` з FFmpeg лишає як запасний candidate. Це важливо, бо на частині машин friendly name працює стабільніше за PnP-style alias, і саме через це раніше доводилося вручну передавати `--device "<camera name>"`.

У `auto` режимі capture supervisor може пройти кілька кандидатів поспіль, якщо попередній device id не стартував без жодного кадру.

## 2. Probe layer

Після резолву device викликається `probe_camera(...)`.

### Навіщо probe взагалі

Probe не запускає стрім, а збирає capability data:

- доступні формати
- максимальні роздільності
- доступні FPS

Далі `resolve_params_from_probe(...)` вибирає "кращий" режим під requested envelope користувача.

### Як обирається best mode

Пріоритет у `CameraMode::score()`:

1. compressed format (`mjpeg`, `h264`)
2. більша роздільність
3. більший FPS

Якщо користувач явно задав `--input-format`, probe не переписує цей вибір.

## 3. Capture attempts

Після probe формується список fallback-спроб.

### V4L2

| Label | Що відкидається |
| --- | --- |
| `primary` | нічого, беремо exact resolved params |
| `no input_format` | даємо драйверу самому домовитися про формат |
| `driver defaults` | прибираємо і size, і fps, і input format |

### DirectShow

| Label | Що відкидається |
| --- | --- |
| `primary` | exact resolved params |
| `no input_format` | без pinned format |
| `driver size` | без `-video_size` |
| `driver defaults` | без size, fps і pinned format |

Повтори однакових arg-lists автоматично видаляються через `dedup_attempts()`.

## 4. FFmpeg args builder

Builder враховує backend.

### Спільна частина

- `-hide_banner`
- `-nostdin`
- `-loglevel warning`
- `-thread_queue_size 4`
- `-an`
- `-flush_packets 1`

### Linux / `v4l2`

- додає `-fflags nobuffer`
- додає `-flags low_delay`
- використовує `-input_format <fmt>` для pinned format

Якщо probe вибрав `mjpeg` і ми не пішли у fallback profile, Observans може робити `-c:v copy -f mjpeg pipe:1` без re-encode.

### Windows / `dshow`

- додає `-rtbufsize 128M`
- для raw format використовує `-pixel_format <fmt>`
- для compressed format використовує `-vcodec <fmt>`

Поточний код спеціально **не** додає `-fflags nobuffer` та `-flags low_delay` на Windows, бо вони спричиняли падіння `dshow`.

## 5. Запуск окремої attempt

`run_capture_attempt(...)` робить:

1. `Command::new(ffmpeg).spawn()`
2. бере `stdout` і `stderr`
3. запускає окремий stderr collector thread
4. запускає окремий stdout reader thread
5. reader thread читає pipe chunk'ами по `8192` байт
6. `JpegStreamParser` витягує повні JPEG frames
7. frames передаються через `mpsc` у головний loop attempt'а

### Чому stdout іде через ще один thread

Так capture loop може:

- не блокуватися назавжди на `read()`
- періодично перевіряти `gate.client_count()`
- швидко вбити FFmpeg, якщо глядачів уже немає

## 6. JPEG parser

`JpegStreamParser` не покладається на довжину пакета. Він шукає JPEG markers:

- SOI: `FF D8`
- EOI: `FF D9`

Поточна поведінка:

- сміття перед першим кадром відкидається
- неповний кадр залишається в buffer
- кілька кадрів в одному chunk теж підтримуються

Є ще helper `jpeg_dimensions(frame)`, який читає JPEG SOF marker, щоб оновити metrics фактичними `width x height`.

## 7. Delivery у broadcast bus

На кожен frame:

1. `metrics.note_frame(...)`
2. `tx.send(frame)`

Кожен клієнт `/stream` має окремий `broadcast::Receiver`.

Якщо receiver відстає, web layer бачить `RecvError::Lagged(skipped)` і додає `queue_drops`.

## 8. Idle stop і release camera

Головний loop attempt'а перевіряє наявність клієнтів двома способами:

- кожні `200 ms` timeout на `recv_timeout`
- кожні `10` кадрів під час активного потоку

Якщо клієнтів більше нема:

1. `child.kill()`
2. drop receiver
3. `reader.join()`
4. `child.wait()`

Саме `wait()` тут критичний: без нього процес може лишитися zombie, а камера - заблокованою.

## 9. Restart/backoff

Якщо attempt або session зламалася не через idle-stop:

- якщо вже були кадри -> retry через `1s`
- якщо кадрів не було -> backoff росте `1 -> 2 -> 3 -> 4 -> 5s`

Кожен retry збільшує `metrics.restarts`.

## 10. Що бачить metrics

Capture pipeline напряму оновлює:

- `stream_input`
- `res`
- `fps_actual`
- `frame_age_ms`
- `avg_frame_kb`
- `restarts`

Тому `/metrics` у браузері показує не просто статичний config, а реальний стан активного capture.
