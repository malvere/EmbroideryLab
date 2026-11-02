# Сборка и запуск

Основная платформа — Windows x64. Ядро написано отдельно от интерфейса; переносимость на другие платформы не означает, что их GUI прошёл приёмку.

## Инструменты

Нужен Rust с Cargo, rustfmt и Clippy. В исходной среде использовался Rust/cargo 1.99.1. Установка: [rustup](https://rustup.rs/).

Для Windows можно использовать MSVC toolchain и Visual Studio Build Tools с C++ либо GNU toolchain с совместимым MinGW-w64 в PATH. Абсолютных путей к компилятору в публичном проекте нет. Дистрибутив MinGW не входит в исходники.

## Проверки

Из корня репозитория подготовьте закреплённый компонент PDFium для Windows x64. Скрипт проверяет SHA-256 архива и DLL, копирует DLL и лицензии рядом с отладочной и релизной сборками. Первая подготовка требует доступа к сети; повторная использует проверенную локальную копию.

```powershell
.\tools\setup-pdfium.ps1
```

Затем выполните проверки:

```powershell
cargo fmt --manifest-path embroidery-engine/Cargo.toml --check
cargo test --manifest-path embroidery-engine/Cargo.toml --all-targets --features capture
cargo clippy --manifest-path embroidery-engine/Cargo.toml --all-targets --features capture -- -D warnings
```

Отдельный ignored-тест установленного внешнего DSB-корпуса требует файлов, которых в репозитории нет. Обычный набор использует синтетические фигуры. Для дополнительного независимого чтения DST/DSB установите `pyembroidery==1.5.1`; переменная `EMBROIDERYLAB_TEST_PYTHON` может указать путь к нужному Python. Без этого дополнительная проверка reader не заменяется вымышленным успешным результатом.

## Бинарники

```powershell
cargo build --manifest-path embroidery-engine/Cargo.toml --release --bins
```

Результат для Windows: `embroidery-engine/target/release/embroidery-lab.exe` и `embroidery-cli.exe`. Для импорта PDF рядом с EXE нужен подготовленный `pdfium.dll` и его лицензии. При GNU-сборке также могут потребоваться runtime DLL компилятора; помощник упаковки проверяет и включает такие зависимости. На другом компьютере используйте распакованный архив целиком. Для 3D-просмотра требуется OpenGL 3.3 или новее.

Windows-помощники выполняют подготовку PDFium автоматически:

```powershell
.\tools\build.ps1 -Release
.\tools\run.ps1 -Release
```

Для запуска отладочной версии:

```powershell
cargo run --manifest-path embroidery-engine/Cargo.toml --bin embroidery-lab
```

Без интерфейса (импорт PDF по-прежнему требует PDFium):

```powershell
cargo test --manifest-path embroidery-engine/Cargo.toml --no-default-features
cargo build --manifest-path embroidery-engine/Cargo.toml --release --no-default-features --bin embroidery-cli
```

## Синтетическая независимая проверка

```powershell
python -m pip install Pillow==13.0.0 pyembroidery==1.5.1
python tools/quality_v05.py --cli embroidery-engine/target/release/embroidery-cli.exe
```

Скрипт создаёт только синтетические входы и результаты в `outputs/quality_v05/`: семь случаев в DST и DSB, проверки отказа без перезаписи, PNG, HTML и summary с SHA. Он не требует пользовательских рисунков или установленных программ вышивки. Рабочие `outputs/` исключены из Git.

Состояние проверок именно публичного экспорта приведено в [VALIDATION.md](VALIDATION.md).

## Упаковка Windows GNU

`tools/package-release.ps1` собирает GUI и CLI, проверяет импорты PE, включает закреплённый PDFium с лицензиями и формирует ZIP, отдельные EXE, `pdfium.dll`, сведения о сборке и SHA256SUMS. Для этого помощника нужны Windows GNU Rust toolchain, MinGW GCC и `objdump` либо LLVM `llvm-readobj` в PATH. Прямая сборка MSVC через Cargo остаётся отдельным вариантом.

```powershell
.\tools\package-release.ps1 -Version 0.6.0 -SourceCommit (git rev-parse HEAD)
```

Помощник не включает пользовательские образцы и результаты локальной оцифровки в архив. Для переноса выпуска 0.6.0 используйте полный ZIP. Отдельный EXE без `pdfium.dll` не поддерживает импорт PDF; остальные форматы не требуют этого компонента. При распространении отдельных файлов сохраняйте лицензии PDFium и других зависимостей из архива.
