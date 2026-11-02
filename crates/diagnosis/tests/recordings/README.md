# Регресійні записи діагностики

Кожен запис — пара файлів:

- `<ім'я>.json.gz` — знеособлений запис гри (кадри PresentMon, метрики, DPC драйверів).
  Імена сторонніх процесів замінено на `app1.exe`, `app2.exe`…; лишаються гра, системні
  процеси Windows і `MsMpEng.exe`; повідомлень PresentMon (можуть містити шляхи) немає.
- `<ім'я>.toml` — що це за сцена і що **має** показати діагностика.

`cargo test -p optimax-diagnosis --test recordings` проганяє всі записи через `diagnose` і
звіряє з очікуваннями. Якщо правка правил змінила вердикт — тест покаже, на якому записі й що
саме.

## Новий запис

PowerShell **від адміністратора**, гра запущена:

```powershell
cargo run --release -p optimax-diagnosis --example fixture -- record re9.exe 60 re9-dlss-quality --desc "RE9, DLSS Quality, 1440p, коридор після старту"
```

Запис починається, щойно гра у фокусі (`--start hotkey` — за Ctrl+Shift+F9). Інструмент
стискає запис, показує вердикти й пише чернетку `<ім'я>.toml`. **Перевір чернетку**:
очікування мають бути правдою для цієї сцени (DLSS Quality на важкій сцені — відеокарта), а не
просто тим, що вийшло. Межі `avg_fps` — ±20 % від запису; `min_confidence` — із запасом 15.

Запис, збережений раніше (`diagnose --save файл.json`): `… --example fixture -- import файл.json <ім'я>`.

Звірка з поясненнями: `… --example fixture -- check [ім'я]`.

## Формат `.toml`

```toml
description = "RE9, DLSS Quality, 1440p"   # що записано
recorded = "2026-11-02"
display_hz = 200                            # для розпізнавання V-Sync

[expect]                                    # порожнє поле не перевіряється
bottleneck = "gpu"                          # gpu | cpu | frame_cap | unclear | insufficient_data
min_confidence = 70                         # впевненість головного вердикту, %
has = ["gpu_bound"]                         # ці вердикти мають бути
absent = ["cpu_bound"]                      # цих не має бути
avg_fps = [110.0, 170.0]                    # середній FPS у межах
low_1pct_fps = [70.0, 130.0]
```

Аудит конфігурації й кеш шейдерів залежать від ПК, тому в цих тестах їх немає.
