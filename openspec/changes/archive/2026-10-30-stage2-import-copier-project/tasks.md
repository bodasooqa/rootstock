## 1. Ядро: файл ответов и пропуск исходников (design D3, D6)

- [x] 1.1 Падающие тесты `copier::parse_answers` в `src/copier.rs`:
  - ответы screener (скаляры, словарь `pi_models`) разбираются, `_src_path` и `_commit` вынуты;
  - нет `_src_path` или `_commit`, не строка, не словарь, иной ключ на `_`, значение `null` — ошибки с именем ключа.

  Проверка: красные.
- [x] 1.2 Падающие тесты пропуска:
  - `TemplateConfig::skip` из `rootstock.toml` не читается: ключ `skip` отвергается как неизвестный;
  - `render_tree` не рендерит исходник из `skip` — ни имя, ни содержимое с неизвестным фильтром не дают ошибки;
  - `copier::convert` кладёт в `config.skip` то же, что в `remove`.

  Проверка: красные.
- [x] 1.3 Реализовать `parse_answers`, `CopierAnswers`, поле `skip`, пропуск в `render_tree`. Проверка: `cargo test --lib` зелёный.

## 2. Края: загрузка стороны и сторона проекта (design D1, D2, D4, D5)

- [x] 2.1 Падающие сквозные тесты в `tests/cli_import_copier.rs`. Шаблон — `TemplateRepo` с тегами в формате Copier: `copier.yml`, `_subdirectory`, шаблон файла ответов, managed- и стартовый файл. Проект — git-репозиторий с `.copier-answers.yml`, закоммиченный. Сценарии:
  - импорт проекта: код 0, запись с источником, версией, коммитом тега, ответами и инвентарём с классами, файл ответов удалён, прочие файлы побайтно те же;
  - стартовый файл, удалённый проектом, в инвентаре; `update` на следующий тег его не создаёт;
  - `_commit` не тег, лишний ключ, грязное дерево, запись уже есть, `copier.yml` и `.copier-answers.yml` вместе — код 1, ничего не изменено;
  - неподдерживаемое в теге — код 1, список в stderr, ничего не изменено;
  - после импорта и коммита `update` на тег с `rootstock.toml` и на тег только с `copier.yml` проходит по таблице, `.copier-answers.yml` не возвращается;
  - `new` на теге только с `copier.yml` — код 1, подсказка `import-copier`.

  Проверка: красные.
- [x] 2.2 Реализовать:
  - выбор стороны в `import_copier::run`;
  - сторону проекта;
  - `Side::load_any` и подсказку в `Side::load`;
  - перенос `require_clean_tree` в `commands/mod.rs`;
  - `update` через `load_any`.

  Проверка: `cargo test` зелёный.

## 3. Документация

- [x] 3.1 `README.md`:
  - быстрый старт: `import-copier` в проекте Copier;
  - описание `import-copier`: сторона проекта, отказ при обоих файлах, `update` со старых тегов Copier;
  - убрать «Converting a Copier-generated project is not supported yet».

## 4. Проверки

- [x] 4.1 `cargo fmt --check`, `cargo clippy --all-targets --all-features -- -D warnings`, `cargo test` — зелёные.
- [x] 4.2 Живая проверка в scratchpad: копии screener и rootstock (оба на harness v0.4.5) с `_src_path`, подменённым на локальный клон harness.
  - `import-copier` проходит, ответы в записи совпадают с файлом ответов.
  - После коммита `update --to` тег клона, где v0.4.5 сконвертирован `import-copier`, проходит. Каждое изменение в дереве названо: отличие рендера rootstock от Copier уходит в `stage2-parity`.
