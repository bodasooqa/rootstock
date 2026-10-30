## 1. Каркас конвертера и ключи шаблона (design D2–D5, D10)

- [x] 1.1 Падающие unit-тесты в `src/copier.rs`:
  - ключи harness (`_subdirectory`, `_templates_suffix`, `_envops`) дают `root`, `suffix`, три настройки Jinja;
  - без `_envops` пишутся умолчания Copier: `keep_trailing_newline = true`, остальные false;
  - без `_subdirectory` и `_exclude` в `exclude` стоит `DEFAULT_EXCLUDE` Copier дословно, с `_exclude` — нет;
  - `_min_copier_version` и `_answers_file` отброшены, `min_rootstock` не выставлен;
  - каждая находка из этого списка копится, и все возвращаются вместе:
    - `_jinja_extensions`, `_migrations`, `_preserve_symlinks`, неизвестный `_ключ`;
    - лишний ключ `_envops` или его небулево значение;
    - Jinja в `_subdirectory` или `_templates_suffix`, пустой суффикс;
    - YAML-тег;
  - вывод начинается с комментария D2 и разбирается `config::parse`.

  Проверка: `cargo test copier` красный ровно на новых тестах.
- [x] 1.2 Реализовать `src/copier.rs`:
  - `pub mod` в `lib.rs`;
  - функция преобразования текста `copier.yml` в пару «текст `rootstock.toml` + `TemplateConfig`» или `Unsupported(Vec<Finding>)`;
  - тип ошибки на `thiserror`.

  Проверка: `cargo test copier` зелёный.

## 2. Правила и задачи (design D6, D7)

- [x] 2.1 Падающие тесты:
  - `_skip_if_exists` → `starter` дословно, с ведущим `/` и с Jinja;
  - обе формы исключения «только при обновлении» с парой отброшены, без пары — находка;
  - `_partials` → `exclude`;
  - задача-словарь с `when` `copy` и `update` → фазы `new` и `update`;
  - задача-строка и задача без `when` → обе фазы;
  - находки для задач: иное `when`, команда-список, `working_directory`, `{{` в команде;
  - `_copier_operation` в иной записи `_exclude` или в `_skip_if_exists` — находка.

  Проверка: красные.
- [x] 2.2 Реализовать. Проверка: `cargo test copier` зелёный.

## 3. Вопросы (design D8)

- [x] 3.1 Падающие тесты:
  - `bool`, `str`, `yaml` переносятся с `help`, `default`, `validator`;
  - `str` со словарём `choices` → `choice` с `{label, value}`, со списком → простые значения;
  - `when: "{{ not (rust and frontend) }}"` → `not (rust and frontend)`, `when: false` → `"false"`;
  - словарь-дефолт `yaml` → таблица TOML;
  - находки:
    - нет `type`;
    - `type: int`;
    - поле `secret`;
    - простая форма `name: value`;
    - `choices` у `bool`;
    - нескалярное значение выбора;
    - `null` в дефолте;
    - `when` с `{%`;
    - `_copier_answers` в `default`;
  - порядок вопросов сохраняется.

  Проверка: красные.
- [x] 3.2 Реализовать. Проверка: `cargo test copier` зелёный.

## 4. Дерево шаблона (design D9, D11, D12)

- [x] 4.1 Падающие тесты на каталогах во временном дереве:
  - `{{ _copier_conf.answers_file }}.jinja` и `{{_copier_conf.answers_file}}.jinja` в корне шаблона попадают в список удаляемого и не дают находки;
  - такой же файл во вложенном каталоге — находка;
  - `_copier_conf` в содержимом шаблонизируемого и нешаблонизируемого файла и в имени каталога — находки с путём;
  - симлинк под корнем — находка;
  - не-UTF-8 файл не сканируется и не падает;
  - `.git` не обходится.

  Проверка: красные.
- [x] 4.2 Реализовать обход корня шаблона в `src/copier.rs`. Результат преобразования получает список удаляемых путей. Проверка: `cargo test copier` зелёный.

## 5. Команда `import-copier` (design D1, D13)

- [x] 5.1 Фикстура `tests/fixtures/copier/harness-v0.4.5.yml` — копия `copier.yml` harness v0.4.5 без правок. Падающие сквозные тесты в `tests/cli_import_copier.rs`:
  - каталог шаблона Copier с `template/{{ _copier_conf.answers_file }}.jinja` → код 0, `rootstock.toml` есть, файл ответов удалён, `copier.yml` на месте, stdout пуст;
  - фикстура harness плюс пустой `template/` → код 0;
  - после импорта, коммита и тега `rootstock new` по шаблону создаёт проект без `.copier-answers.yml`;
  - уже есть `rootstock.toml` → код 1, файлы не изменены;
  - оба файла, `copier.yml` и `copier.yaml` → код 1;
  - нет `copier.yml` → код 1;
  - несколько находок → код 1, все названы в stderr, `rootstock.toml` нет, файл ответов на месте.

  Проверка: красные.
- [x] 5.2 Реализовать:
  - `src/commands/import_copier.rs`: запись через временный файл и переименование без перезаписи, затем удаление;
  - подкоманда `ImportCopier` в `src/cli.rs` с текстом помощи.

  Проверка: `cargo test` зелёный.

## 6. Документация

- [x] 6.1 `README.md`:
  - строка `rootstock import-copier` в быстром старте;
  - пункт о стороне шаблона;
  - карта: `src/copier.rs`, `commands/import_copier.rs`.

  Таблица модулей в `CLAUDE.md`: `src/copier.rs` в ядре, `import_copier.rs` в командах.

## 7. Проверки

- [x] 7.1 `cargo fmt --check`, `cargo clippy --all-targets --all-features -- -D warnings`, `cargo test` — зелёные.
