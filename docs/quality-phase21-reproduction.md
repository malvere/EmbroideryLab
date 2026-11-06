# Воспроизведение этапа 2.1

Этот документ описывает локальную проверочную сессию. В публичном репозитории доступны исходники, тесты и проверочные инструменты; частные изображения, их производные, бинарники и каталоги `outputs/` не публикуются. Указанные пути и SHA256 обозначают локальные доказательства, а не доступные для скачивания файлы. Сборка приложения не требует этих частных данных: см. [BUILDING.md](../BUILDING.md).

Все результаты хранятся локально в `C:\EmbroideryLab\outputs\quality-phase21-20261106`.
Эта папка исключена из Git. Исторические `quality-phase2-20261106` не
перезаписывались. `quality_phase21_run.py freeze` уже выполнен, повторный
вызов отказывается заменять исходный снимок.

## Входы и среда

Rust/Cargo 1.99.1, Windows GNU, debug-сборки; Python 3.12.14, Pillow 13.0.0,
PyEmbroidery 1.5.1. Новых runtime-зависимостей нет. `initial/state.json`
содержит исходные хэши/статус, параметры и снимок source/tests/Cargo. Три
замороженных манифеста этапа 2 и соседние `fixture-filehashes.json` проверяются
существующим `load_frozen_manifest`; исходники/эталоны/пороги не рисуются заново.
Seed старого корпуса — 2601106. Серый tuning и синий holdout заморожены
дополнительно до реализации, эталоны — независимо нарисованная точная RGBA
геометрия. Во время выбора кандидата holdout не использовался.

Формат герба определён как WebP по сигнатуре, исходник 500×669. Его исходный
SHA256 совпал с этапом 2. Для всех трёх вариантов герба: 100×133,8 мм,
383×512, палитра 8, TransparentOnly, min pixels 1, min area 0,1 мм²,
simplify 0,1 мм, smoothing/denoise 0, pinned пусто, общие границы и
автоклассификация включены. Изменяется только подготовка. Настройки генераторов,
буфер нити 0,30 мм и шаг Tatami 0,40 мм не подбирались.

В первоначально проверенном исследовательском checkout объект публичного Git `59b0dd641a387aa87cba0b4335eda4d58a2c9300` локально
отсутствует; remote не настроен. Фактический HEAD до работы —
`9a5729f7180c08e7f44929465e694e0517f79302`. Это ограничение отдельно
зафиксировано, а воспроизведение текущего Standard доказано на всех 19 случаях.

При публикации обнаружена отдельная публичная копия на `59b0dd…`; 93 общих файла движка совпали с исходной локальной базой. Этот публичный checkout используется для целевого коммита, без переноса исследовательской истории.

## Фактически выполненные команды

Основные Python-вызовы из `C:\EmbroideryLab` (полные команды и исходные
stdout/stderr рядом с каждым результатом):

```powershell
$taskPython = 'C:\Users\Admin\.cache\codex-runtimes\codex-primary-runtime\dependencies\python\python.exe'
$env:PYTHONPATH = 'C:\EmbroideryLab\outputs\releases\v0.6.0\smoke\python-deps'
$env:PYTHONIOENCODING = 'utf-8'
& $taskPython tools/quality_phase21_run.py freeze
& $taskPython tools/quality_phase21_run.py microfixtures
& $taskPython tools/quality_phase21_run.py run --label standard --cli outputs/quality-phase21-20261106/initial/standard.exe --profile Standard
& $taskPython tools/quality_phase21_run.py equivalence
& $taskPython tools/quality_phase21_run.py run --label candidate --cli outputs/quality-phase21-20261106/candidate-final.exe --profile Contextual
& $taskPython tools/quality_phase21_run.py diagnostic --cli outputs/quality-phase21-20261106/candidate-final.exe --label standard
& $taskPython tools/quality_phase21_run.py diagnostic --cli outputs/quality-phase21-20261106/candidate-final.exe --label candidate
& $taskPython tools/quality_phase21_rag_report.py
& $taskPython tools/quality_phase21_reviews.py --candidate-root outputs/quality-phase21-20261106/candidate
& $taskPython tools/quality_phase21_reviews.py --crest --candidate-root outputs/quality-phase21-20261106/candidate
& $taskPython tools/quality_phase21_reviews.py --resolution --cli outputs/quality-phase21-20261106/initial/standard.exe --profile Standard
& $taskPython tools/quality_phase21_reviews.py --resolution --cli outputs/quality-phase21-20261106/candidate-final.exe --profile Contextual
& $taskPython tools/quality_phase21_run.py summary
```

Для старых 11 REVIEW выполнены `--self-test` и 33 минимальных однопиксельных
воспроизведения назначения метки с замороженной наблюдаемой палитрой. Это
минимальные тесты стадии меток, не проверка контекстного слияния.

Калибровка: `quality_phase21_coverage.py` без аргументов — шесть контрольных
геометрий; `--aa-only` — известные цветные растеризации. `analyze_folder`
выполнен для Legacy/Standard/Contextual герба с одинаковыми factors 1,2 и
четырьмя фиксированными фазами; для серой пары — factors 1,2,4 и независимый
truth. Параметры и SHA256 записаны в `coverage/`. Полные вызовы операций,
изменённой пары и цветного holdout записаны в их `summary.json` и
`operations/*/sewout-pair/acceptance-before-run.json`.

`quality_phase21_real_geometry.py` проверяет сохранённые ELAB по обычным
контурам и исходной геометрии Running/Satin, отдельно от реконструкции
порогов из меток. Все 170116 обязательных центров и 24 новых целевых RGB
подтверждены в конечной геометрии. Полные данные — `real-geometry.json`.

Сборки/проверки из `embroidery-engine`:

```powershell
$env:PATH = 'C:\Users\Admin\.cargo\bin;C:\EmbroideryLab\tools\winlibs\mingw64\bin;' + $env:PATH
$env:CARGO_INCREMENTAL = '0'
cargo fmt
cargo fmt --check
cargo test --offline --no-default-features --lib preparation
cargo test --offline --features capture
cargo clippy --offline --features capture --all-targets -- -D warnings
cargo build --offline --features capture --bins
```

Первый build получил `gcc.exe not found` из-за неполного PATH; повтор с
локальным winlibs успешен. Два ранних фильтрованных вызова `raster_preparation`
дали **0 тестов**, поскольку модуль называется `preparation`; они не считаются
проверкой поведения. Правильный фильтр выполнил 13 содержательных тестов.

Первый полный test дошёл до 52 lib + 129 успешных GUI-тестов и одного отказа
`workspace_save_preserves_language_and_does_not_overwrite_invalid_json`:
Windows Access denied при атомарной замене в системном Temp. Повтор выполнялся
с разрешением выхода из песочницы и отдельными TEMP/TMP в
`outputs/quality-phase21-20261106/test-temp`, без смены пользовательских настроек.
Первый clippy обнаружил увеличение варианта сообщения worker из-за встроенной
диагностики; данные перенесены в Box с прежним JSON. Последующий clippy также
исправил вложенный if. Эти исходные отказы сохранены, а не удалены из истории.

Финальные команды/коды находятся в `fmt-delivery.*`, `test-final.*`,
`clippy-delivery.*`, `build-delivery.*`; агрегат проверки — `final-checks.json`.
Исходная ошибка теста не записывается как PASS. Сборки с новым кодом и
результаты их генерации сверяются отдельно `quality_phase21_source_check.py`:
сначала `candidate-verified`, затем `candidate-delivery`. Предыдущие результаты
сохраняются. Полный план сравнивается без удаления полей; возможные различия
диагностических предупреждений выводятся явно, а не нормализуются молча.

## Повтор без замены сохранённых доказательств

Для нового запуска задавайте новую папку `--output` существующему инструменту
`quality_phase2_validation.py`, передавая прежний `--manifest`. Например:

```powershell
& $taskPython tools/quality_phase2_validation.py `
  --manifest outputs/quality-phase2-20261106/comparison/fixtures.json `
  --output outputs/quality-phase21-20261106/repeat-01/comparison `
  --baseline outputs/quality-phase21-20261106/initial/legacy.exe `
  --candidate outputs/quality-phase21-20261106/initial/standard.exe `
  --diagnostic-cli outputs/quality-phase21-20261106/initial/standard.exe `
  --profile Standard
& $taskPython tools/quality_phase2_validation.py `
  --manifest outputs/quality-phase2-20261106/comparison/fixtures.json `
  --output outputs/quality-phase21-20261106/repeat-01-contextual/comparison `
  --candidate outputs/quality-phase21-20261106/candidate-delivery.exe `
  --profile Contextual
```

Повторите для comparison-unpinned и comparison-paired. У старого инструмента
есть историческое переиспользование точного Legacy-герба; отдельная свежая
генерация Legacy-герба проверена source-check, а не объявлена повтором по копии.
Не удаляйте исходные fixture locks и не запускайте перерисовку эталонов.

## Измерения затрат и непроверенное

`comparison-table.json/csv` содержат измеренные секунды, peak working set,
метод опроса Windows и фактические source/trace размеры для каждого запуска.
Legacy-герб в старом runner переиспользует сохранённый результат: его время
не выдумано, помечено как отсутствующее. Отдельная свежая Legacy-генерация
имеет своё время в source-check. Условия: общая Windows VM под переменной
нагрузкой, debug Rust, одновременно работали независимые аудиторы и сборки;
есть ожидание Cargo lock и холодная компиляция зависимостей. Эти записи не
доказывают ускорение. Сравнивать CLI/trace/generation как одинаковые операции
или длительность approval с CPU-временем алгоритма нельзя.

Windows GUI мышью/системным диалогом не проверен. Использован навык
[computer-use](C:/Users/Admin/.codex/plugins/cache/openai-bundled/computer-use/26.1029.70936/skills/computer-use/SKILL.md),
инвентаризация приложений была доступна; `sky.launch_app` новой сборки вернул
`Computer Use app approval timed out`. Это тайм-аут разрешения инструмента,
не успешный запуск и не результат теста мышью. Программные egui/worker тесты
классифицированы отдельно. Обход этого разрешения другим способом запуска
не выполнялся.

Физическая отшивка, реальные пяльцы/контроллер/обрезки/материалы не проверены.
Форма наблюдений пуста. Есть неоднозначные цветные AA-края, старые ложные
цветовые островки и утечки маски, все 11 REVIEW; автоматической семантической
истины для герба нет. Режим Contextual не включён по умолчанию.

## Финальные результаты проверок

Все четыре финальных exit-кода равны 0: fmt, полный test, clippy с `-D warnings`,
build с GUI/capture. Полный test: 421 успешно, 0 отказов, 1 ignored в 35 наборах.
Пропущен только `installed_corpus_preserves_all_record_semantics`, которому
нужен установленный сторонний DSB-корпус; он не запускался и не считается PASS.
Первоначальные Access denied и clippy-отказы сохранены в отдельных логах.

CLI `candidate-delivery.exe`, SHA256
`492a48d8388ce0294de2a3f355581d633acec27785fd52adc5126bc7b115b230`,
дала полное побайтовое совпадение всех 19 случаев (DST, стежки, ELAB и весь план)
без различий предупреждений. Четыре Legacy/старого ручного ELAB проверки и оба
контекстных образца также совпали. Доказательства:
`source-equivalence/candidate-delivery/summary.json` и
`micro-contextual-pair/summary.json` рядом с ним.

Финальные SHA256 исходника, Cargo, тестов, инструментов и бинарников —
`final-source-manifest.json`; исходники сохранены в `final-source/`.
Дополнительный `input-archive/` содержит проверенные побайтовые копии входов
и эталонов. Эти копии собраны после проверок при подтверждённых прежних
fixture-lock хэшах; время позднего копирования записано явно, оно не выдаётся
за новую исходную фиксацию. Оригиналы не изменены.

Небольшой пакет для проверки владельцем:
`Standard-vs-Contextual-12_8mm.zip`, SHA256
`40d4af8a3392b1b30181332c7d7952595f12eca05898d7fd86ca654dec9c1972`.
В нём исходный серый мотив, параметры, Standard/Contextual ELAB и DST,
сопоставимые фрагменты и незаполненные физические наблюдения. Цветовой holdout
использует другую палитру на той же геометрии и координатах; пространственная
независимость этих двух маленьких мотивов не заявляется.
