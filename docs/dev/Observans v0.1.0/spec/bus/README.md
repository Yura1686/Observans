# Bus

- Date:   **2026-04-07**
- Status: **Затведжено**
- Note:   **Цей файл лишає в `spec/bus` конкретні посилання на актуальні матеріали з `spec/main`, які описують `observans-bus` і роль `ClientGate`.**

## Основні посилання на `main`

| Файл у `main`                                              | Для чого відкривати                                         |
| ---------------------------------------------------------- | ----------------------------------------------------------- |
| [../main/architecture.md](../main/architecture.md)         | Місце `observans-bus` у загальній архітектурі               |
| [../main/crates.md](../main/crates.md)                     | Опис `observans-bus/src/lib.rs`, `broadcast` і `ClientGate` |
| [../main/startup-flow.md](../main/startup-flow.md)         | Коли і як `ClientGate` будить capture thread                |
| [../main/capture-pipeline.md](../main/capture-pipeline.md) | Delivery кадрів, idle-stop, взаємодія bus і capture         |

## Що саме сюди входить

`spec/bus` відповідає за теми навколо:

- `FrameSender`
- `FrameReceiver`
- `broadcast::channel`
- `ClientGate`

## Пряме посилання на код

- [`observans-bus/src/lib.rs`](../../../../../observans-bus/src/lib.rs)
