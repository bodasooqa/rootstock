## 1. Сборка тестов под Windows и CI

- [x] 1.1 Сделать так, чтобы тесты собирались под `x86_64-pc-windows-msvc`. `TemplateRepo::symlink` — платформенным вызовом. Исполняемые файлы фикстуры получают бит через индекс (`git update-index --chmod=+x`) на всех ОС (design D6). Проверка: `cargo clippy --target x86_64-pc-windows-msvc --all-targets -- -D warnings` чисто, тесты на macOS зелёные.
- [x] 1.2 Добавить в `.github/workflows/ci.yml` джобу `test` с матрицей `windows-latest`, `macos-latest`: `cargo test --locked`, а на Windows ещё clippy с `-D warnings` (design D6). Проверка: YAML разбирается, джоба видна в PR.

## 2. Режимы из дерева git и выгрузка без autocrlf

- [x] 2.1 Написать падающие тесты (design D6):
  - файл `100755` в дереве, `0644` на диске выгрузки — выпускается исполняемым;
  - ссылка `120000`, выгруженная файлом при `core.symlinks=false`, — выпускается ссылкой;
  - `core.autocrlf=true` в окружении git — LF-файл выпускается байт в байт.

  Проверка: тесты красные на текущем коде, кроме тех, что текущий код уже проходит на этой ОС, — их назвать в отчёте.
- [x] 2.2 Реализовать design D1 и D2 в `src/source.rs` и `src/render.rs`: `Checkout::modes`, `ls-tree` и `ls-files`, параметр режимов в `render_tree`, `--config core.autocrlf=false` у `clone`. Проверка: тесты 2.1 и все прежние зелёные.

## 3. Бит исполнения проекта и ссылки на Windows

- [x] 3.1 Реализовать design D3 в `src/apply.rs` и `src/commands/new.rs`: бит проекта из индекса на не-Unix, `set_index_modes`, вызов из `apply` и после `git init`. Тест, который на не-Unix проверяет `100755` в индексе после `new --git`. Проверка: clippy под Windows чисто, тесты на macOS зелёные.
- [x] 3.2 Реализовать design D4 в `src/apply.rs`: проба ссылки до записи, ссылки последними, `symlink_dir` или `symlink_file`. Юнит-тест на текст ошибки пробы. Проверка: clippy под Windows чисто.

## 4. sh для хуков

- [x] 4.1 Реализовать design D5 в `src/hooks.rs` и командах: поиск оболочки среди проверок до записи, путь передаётся запуску. Сквозной тест с POSIX-хуком (`if command -v true …`) на всех ОС. Проверка: тест зелёный на macOS, clippy под Windows чисто.

## 5. Документация и прогон на Windows

- [x] 5.1 README: поддержка Windows — `sh` из Git for Windows, бит в индексе, ссылки в режиме разработчика. Проверка: README называет все три пункта.
- [x] 5.2 Исправления по ревью и первому прогону Windows CI — по обновлённому design (D1–D6). Проверка: находки закрыты тестами, проверки на macOS зелёные, clippy под Windows чисто.
- [ ] 5.3 Пуш ветки и зелёная джоба `test` на `windows-latest` и `macos-latest`. Красное чинится в этом же change. Проверка: `gh pr checks` зелёный.
