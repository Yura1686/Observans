# Core

- Date:   **2026-04-07**
- Status: **Затведжено**
- Note:   **Цей файл лишає в `spec/core` конкретні посилання на актуальні матеріали з `spec/main`, які описують `observans-core`.**

## Основні посилання на `main`

| Файл у `main` | Для чого відкривати |
| --- | --- |
| [../main/architecture.md](../main/architecture.md) | Загальна архітектура `observans-core`, inventory, probe, metrics, logs, shutdown |
| [../main/crates.md](../main/crates.md) | Детальний розбір файлів `observans-core/src/*` |
| [../main/startup-flow.md](../main/startup-flow.md) | Bootstrap, camera picker, dashboard, startup sequence |
| [../main/capture-pipeline.md](../main/capture-pipeline.md) | Capture supervisor, FFmpeg attempts, JPEG parser, retry/backoff |

## Що саме сюди входить

`spec/core` відповідає за теми навколо:

- `config.rs`
- `bootstrap.rs`
- `platform.rs`
- `runtime.rs`
- `camera_inventory.rs`
- `probe.rs`
- `capture.rs`
- `metrics.rs`
- `sensors.rs`
- `logs.rs`
- `shutdown.rs`
- `tui.rs`

## Прямі посилання на код

- [`observans-core/src/lib.rs`](../../../../../observans-core/src/lib.rs)
- [`observans-core/src/config.rs`](../../../../../observans-core/src/config.rs)
- [`observans-core/src/bootstrap.rs`](../../../../../observans-core/src/bootstrap.rs)
- [`observans-core/src/camera_inventory.rs`](../../../../../observans-core/src/camera_inventory.rs)
- [`observans-core/src/probe.rs`](../../../../../observans-core/src/probe.rs)
- [`observans-core/src/capture.rs`](../../../../../observans-core/src/capture.rs)
- [`observans-core/src/metrics.rs`](../../../../../observans-core/src/metrics.rs)
- [`observans-core/src/logs.rs`](../../../../../observans-core/src/logs.rs)
- [`observans-core/src/shutdown.rs`](../../../../../observans-core/src/shutdown.rs)
- [`observans-core/src/tui.rs`](../../../../../observans-core/src/tui.rs)
