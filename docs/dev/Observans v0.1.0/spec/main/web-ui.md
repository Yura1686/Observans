# Web UI

- Date:   **2026-04-07**
- Status: **Затверджено**
- Note:   **Актуалізований файл документації для поточного стану `web-ui Observans v0.1.0`.** 

## Frontend-модель

UI не збирається окремим frontend toolchain. Він повністю вбудовується в binary:

```text
observans-web/assets/
├── index.html
├── styles.css
└── app.js
```

[`observans-web/src/ui.rs`](../../../../../observans-web/src/ui.rs) просто вставляє CSS і JS у HTML-шаблон через `replace()`.

## Поточний layout

### `index.html`

Основні секції:

```text
.shell
├── .masthead
│   └── .brand-title
└── .layout
    ├── .main-column
    │   ├── .stream-card
    │   │   ├── #stream-frame
    │   │   │   ├── canvas#stream-stage
    │   │   │   ├── img#stream-source
    │   │   │   ├── #stream-placeholder
    │   │   │   └── #fullscreen-btn
    │   │   └── .stream-footer
    │   └── .telemetry-card
    │       ├── CPU bar
    │       ├── RAM bar
    │       └── TEMP bar
    └── .side-column
        ├── .summary-card
        │   ├── host / clients
        │   ├── battery meter
        │   ├── stream badges
        │   └── recording controls
        └── .info-card
            ├── clock / date
            ├── platform / uptime
            ├── resolution / input
            └── pipeline footnote
```

### IDs, які вже зафіксовані тестами

- `record-btn`
- `stop-btn`
- `save-btn`
- `stream-stage`
- `fullscreen-btn`
- `battery-fill`

Ці контракти перевіряються тестами в [`observans-web/tests/http.rs`](../../../../../observans-web/tests/http.rs).

## Візуальна мова

[`observans-web/assets/styles.css`](../../../../../observans-web/assets/styles.css) використовує twilight-напрям:

- темно-синій багатошаровий sky gradient
- м'який "sunset glow"
- напівпрозорі glass-панелі
- теплі accent-стани для recording і live status

Основні CSS-патерни:

- `.panel` - базова glass-card
- `.stream-frame.is-live` - переключає canvas із placeholder у live state
- `.chip`, `.badge` - компактні pills
- `.telemetry-fill` - прості bar indicators замість SVG graph-ів
- `.recording-dot.live` - активний recording indicator

## Browser-side stream rendering

Поточний JS не малює MJPEG напряму в DOM. Замість цього:

1. hidden `<img>` тягне `/stream`
2. `onload` означає, що новий JPEG уже доступний
3. `requestAnimationFrame` постійно перемальовує `<img>` у `<canvas>`

Навіщо це потрібно:

- `canvas.captureStream()` для локального запису
- повний контроль над fullscreen-сценарієм
- чіткий перехід між placeholder і live-state

## `app.js` - стан і сценарії

### Основний runtime state

У [`observans-web/assets/app.js`](../../../../../observans-web/assets/app.js) тримаються:

- `streamAlive`
- `lastMetricsOk`
- `reconnectTimer`
- `renderLoopId`
- `lastMetrics`
- `mediaRecorder`
- `recordedChunks`
- `recordedBlob`
- `recordedUrl`
- `recordedMimeType`
- `recordStartedAt`
- `recordTicker`

### Підключення до стріму

`connectStream()`:

- скидає reconnect timer
- переводить UI в non-live state
- ставить status `connecting stream`
- виставляє `streamSource.src = withTs("/stream")`

`withTs()` додає query-параметр часу, щоб уникнути cache reuse.

### Рендер loop

`renderFrame()`:

- підлаштовує canvas size під `naturalWidth/naturalHeight`
- викликає `drawImage(...)`
- працює через `requestAnimationFrame`

### Reconnect

На `streamSource.onerror`:

- стрім позначається як мертвий
- render loop зупиняється
- ставиться статус `reconnecting stream`
- повторне підключення планується через `1500 ms`

## Метрики в UI

`tick()` кожну секунду викликає `/metrics` і розкладає snapshot по DOM.

### Приклади мапінгу

| Поле metrics | DOM / ефект |
| --- | --- |
| `cpu` | `#cpu`, `#cpu-bar-fill` |
| `ram_pct` | `#ram`, `#ram-bar-fill` |
| `ram_used_mb`, `ram_total_mb` | `#ram-sub` |
| `temp` | `#temp`, `#temp-bar-fill`, `#temp-sub` |
| `batt`, `batt_status` | `#batt`, `#batt-sub`, `#battery-fill` |
| `hostname` | `#host` |
| `clients` | `#clients` |
| `platform_name`, `capture_backend` | `#host-sub`, `#backend-pill` |
| `uptime` | `#uptime` |
| `res` | `#res`, `#video-res` |
| `fps_actual`, `fps_target` | `#fps` |
| `frame_age_ms` | `#frame-age`, `#stream-meta` |
| `stream_input` | `#stream-input`, `#video-meta-line` |
| `stream_pipeline` | `#stream-pipeline` |
| `avg_frame_kb` | `#frame-size-pill`, `#video-meta-line` |
| `restarts` | `#restart-pill`, `#stream-meta` |

Якщо `/metrics` падає більше ніж на 4 секунди, UI переходить у `telemetry unavailable`.

## Browser recording

Локальний запис працює через `MediaRecorder` над `stage.captureStream(...)`.

### Формати

JS пробує:

1. `video/webm;codecs=vp9`
2. `video/webm;codecs=vp8`
3. `video/webm`

### Коли можна записувати

Кнопка `Start` стає активною тільки якщо:

- stream живий
- canvas має валідні розміри
- браузер підтримує `MediaRecorder`
- браузер підтримує `canvas.captureStream`

### Збереження

Файл зберігається як:

```text
observans-YYYY-MM-DD-HH-MM-SS.webm
```

або `.mp4`, якщо recorder реально поверне mp4 mime type.

## Fullscreen

Fullscreen прив'язаний не до всього `body`, а до `#stream-frame`.

Поточна поведінка:

- клік по `#fullscreen-btn` викликає `requestFullscreen()`
- `fullscreenchange` синхронізує aria-label і CSS state

## Visibility handling

Коли вкладка ховається:

- якщо запис не ведеться, стрім відключається
- render loop зупиняється

Коли вкладка повертається:

- `connectStream()` запускається знову

Це зменшує зайве навантаження, але не ламає сценарій локального запису.
