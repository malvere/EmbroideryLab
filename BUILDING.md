# Сборка и запуск

Основная платформа — Windows x64. Ядро написано отдельно от интерфейса; переносимость на другие платформы не означает, что их GUI прошёл приёмку.

## Инструменты

Нужен Rust с Cargo, rustfmt и Clippy. В исходной среде использовался Rust/cargo 1.99.1. Установка: [rustup](https://rustup.rs/).

Для Windows можно использовать MSVC toolchain и Visual Studio Build Tools с C++ либо GNU toolchain с совместимым MinGW-w64 в PATH. Абсолютных путей к компилятору в публичном проекте нет. Дистрибутив MinGW не входит в исходники.

## Проверки

Из корня репозитория:

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

Результат для Windows: `embroidery-engine/target/release/embroidery-lab.exe` и `embroidery-cli.exe`. При GNU-сборке runtime DLL компилятора могут потребоваться рядом с EXE; архив выпуска должен включать их и лицензии. На другом компьютере проверяйте распакованный архив целиком.

Для запуска отладочной версии:

```powershell
cargo run --manifest-path embroidery-engine/Cargo.toml --bin embroidery-lab
```

Без интерфейса:

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

`tools/package-release.ps1` собирает GUI и CLI, проверяет импорты PE, добавляет лицензии зависимостей и формирует ZIP, отдельные EXE и SHA256SUMS. Для этого помощника нужны Windows GNU Rust toolchain, MinGW GCC и `objdump` либо LLVM `llvm-readobj` в PATH. Прямая сборка MSVC через Cargo остаётся отдельным вариантом.

```powershell
.\tools\package-release.ps1 -Version 0.5.0 -SourceCommit (git rev-parse HEAD)
```

Помощник не включает пользовательские образцы и результаты локальной оцифровки в архив. Проверенный выпуск 0.5.0 не требует дополнительных runtime DLL; ZIP дополнительно содержит документацию и лицензии.
