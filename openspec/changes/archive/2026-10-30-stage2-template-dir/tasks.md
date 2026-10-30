## 1. Подключения от каталога шаблона (design D5, D6)

- [x] 1.1 Падающие тесты:
  - в `src/config.rs` `rootstock.toml` с ключом `include_root` отклоняется как неизвестный ключ;
  - в `src/render.rs` файл под `root = "template"` подключает `template/_partials/x.jinja` от каталога шаблона;
  - `RootOutside` говорит «outside the template directory».

  Проверка: `cargo test` красный ровно на новых тестах.
- [x] 1.2 Удалить `TemplateConfig.include_root` и `render::include_root` с его тестами. `Side::load` передаёт в `Jinja::new` каталог checkout'а; doc-комментарии `jinja.rs` и `config.rs` переписать; текст `RootOutside` поправить. Проверка: `cargo test` зелёный, `git grep include_root -- src tests` находит только тесты отказа от ключа.

## 2. Подкаталог в источнике (design D1, D3, D4)

- [x] 2.1 Падающие тесты в `src/source.rs` и `tests/source.rs`:
  - разбор `subdir`: нормализация завершающего `/`; отказ для пустого, абсолютного, `.`, `..`, пустого компонента, `\`, `:`, NUL;
  - checkout по тегу удалённого (`file://`) источника с `subdir`;
  - checkout локального `HEAD` с `subdir`;
  - локальный путь в подкаталог плюс `subdir` даёт `a/b`;
  - подкаталог без `rootstock.toml` или через симлинк — ошибка с итоговым путём в сообщении.

  Проверка: тесты красные.
- [x] 2.2 Реализовать разбор `subdir` и параметр подкаталога у `checkout`; локальный `HEAD` идёт через `template_dir`. Проверка: `cargo test` зелёный.

## 3. Запись (design D2)

- [x] 3.1 Падающие тесты в `src/record.rs`:
  - запись с `subdir` проходит туда и обратно;
  - запись без поля читается с `subdir = None`, а при записи поле не появляется;
  - старая фикстура записи читается без изменений.

  Проверка: тесты красные.
- [x] 3.2 Добавить необязательное поле `subdir` в `Record`/`RawRecord`. Проверка: `cargo test` зелёный.

## 4. CLI и команды

- [x] 4.1 Падающие сквозные тесты в `tests/cli_new_update.rs`:
  - `new <file-url> --subdir tpl` → проект и `subdir = "tpl"` в записи;
  - `update` на следующий тег приносит правку из `tpl/`;
  - `new <repo> --subdir tpl --version HEAD`;
  - `--subdir ../x` → код 1, каталог не создан;
  - `--subdir docs` без `rootstock.toml` → код 1, каталог не создан;
  - целевой тег без `tpl/rootstock.toml` → `update` код 1, файлы и запись не изменены;
  - `status` проекта с `subdir` работает.

  Проверка: тесты красные.
- [x] 4.2 Флаг `new --subdir`; `update` и `status` разбирают `subdir` из записи и передают в `checkout`. Проверка: `cargo test` зелёный.

## 5. Документация и проверки

- [x] 5.1 `README.md`: `--subdir` в быстром старте; абзац о формате шаблона — подкаталог любого источника, подключения от каталога с `rootstock.toml`. Проверка: `git grep -n include_root README.md` пуст.
- [x] 5.2 Полные проверки:
  - `cargo fmt --check`;
  - `cargo clippy --all-targets --all-features -- -D warnings`;
  - `cargo test`.
