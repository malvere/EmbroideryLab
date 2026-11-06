# Воспроизведение сравнения Phase 2

Для Contextual см. [воспроизведение этапа 2.1](quality-phase21-reproduction.md). Ниже сохранены команды прежней сессии.

Это точная инструкция для исходной локальной сессии. Частные изображения, замороженные манифесты и `outputs/` не входят в публичный Git-репозиторий. В чистом checkout синтетические входы можно создать через `quality_phase2_validation.py --prepare-only`; для повторения частных случаев нужны их исходники и локальные манифесты. Основная сборка приложения не зависит от этих данных.

Рабочая папка — `C:\EmbroideryLab`. Основная реализация остаётся в Rust;
Python нужен только независимым проверкам. Использовались Python 3.12.14,
Pillow 13.0.0 и PyEmbroidery 1.5.1.

19 пар заданы тремя уже сохранёнными наборами:

- `outputs/quality-phase2-20261106/comparison/fixtures.json`: 12 случаев;
- `outputs/quality-phase2-20261106/comparison-unpinned/fixtures.json`: 4 случая;
- `outputs/quality-phase2-20261106/comparison-paired/fixtures.json`: 3 физических
  размера одного и того же PNG.

Рядом с каждым находится `fixture-filehashes.json`: SHA256 самого манифеста,
исходников и эталонных PNG, канонических настроек и допусков, seed и размеры.
Хэши исходников также сверены с сохранёнными результатами обоих CLI.
`--manifest` проверяет эти хэши и читает существующие случаи. Растры, эталоны
и допуски не генерируются заново. Изменение любого из них прекращает запуск.
Выбор уже существующего `--output` тоже читает его замороженный набор:
пропущенный флаг `--unpinned`/`--paired` больше не может перерисовать входы.

Одна команда PowerShell воспроизводит все 19 пар с исходной сборкой и
поставленной финальной сборкой. Перезаписываются только результаты этих
проверок в указанных каталогах; входы и эталоны проверяются перед запуском.

```powershell
$taskPython = 'C:\Users\Admin\.cache\codex-runtimes\codex-primary-runtime\dependencies\python\python.exe'
$env:PYTHONPATH = 'C:\EmbroideryLab\outputs\releases\v0.6.0\smoke\python-deps'
$env:PYTHONIOENCODING = 'utf-8'
$taskPhase = 'C:\EmbroideryLab\outputs\quality-phase2-20261106'
$taskBaseline = Join-Path $taskPhase 'baseline-bin\embroidery-cli-source.exe'
$taskCandidate = Join-Path $taskPhase 'final-bin\embroidery-cli.exe'
foreach ($taskGroup in @('comparison', 'comparison-unpinned', 'comparison-paired')) {
    $taskOutput = Join-Path $taskPhase $taskGroup
    & $taskPython tools/quality_phase2_validation.py `
        --manifest (Join-Path $taskOutput 'fixtures.json') `
        --output $taskOutput `
        --baseline $taskBaseline --candidate $taskCandidate `
        --diagnostic-cli $taskCandidate --profile Standard
    if ($LASTEXITCODE -ne 0) { throw "Validation failed: $taskGroup" }
}
& $taskPython tools/quality_phase2_report.py --root $taskPhase
if ($LASTEXITCODE -ne 0) { throw 'Report generation failed' }
```

Для быстрой проверки манифестов без расчёта стежков выполните тот же цикл
с аргументами `--manifest ... --prepare-only`. Он должен показать 12, 4 и 3.
Сохранённый baseline герба повторно используется только после проверки
исходника; его DST, ELAB, план и стежки побайтово совпали с результатом
сборки исходного снимка. Доказательство находится в
`baseline-source-equivalence-corrected/equivalence.json`.

Дополнительные воспроизводимые проверки:

```powershell
& $taskPython tools/autodigitize_validation.py --candidate $taskCandidate `
    --output (Join-Path $taskPhase 'legacy-corpus')
& $taskPython tools/quality_phase2_density.py
& $taskPython tools/quality_phase2_real_roundtrip.py --cli $taskCandidate
& $taskPython tools/quality_phase2_real_segments.py
```

Итоговые файлы: `comparison-table.csv`, `comparison-table.json`,
`quality-summary.json`, `comparison/contact-sheet.png` и HTML каждого набора.
В таблицах указаны обе физические оси, размеры исходника, реальные габариты
стежков, пути и SHA256 настроек. Подготовка Standard экспериментальная;
Legacy сохранён по умолчанию. Структурные, геометрические и машинные проверки
не являются подтверждением качества физической отшивки.
