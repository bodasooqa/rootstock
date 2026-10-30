## Why

Этап 2 — «Copier import and parity» (ТЗ: §Stages, п. 2). Первая доработка этапа, каталог шаблона и `--subdir`, влита. Следующая — `import-copier`, без которого harness не перейдёт на rootstock.

Change берёт сторону шаблона (ТЗ: §import-copier, «Template side»). Сторона проекта строится на ней: она конвертирует `copier.yml` нужного тега в памяти по тем же правилам. Поэтому преобразование и список неподдерживаемого должны появиться раньше.

## What Changes

- **Команда `rootstock import-copier`** в каталоге шаблона Copier (ТЗ: §Commands, §import-copier).
  - Читает `copier.yml` и пишет рядом `rootstock.toml`; после этого Copier не нужен (ТЗ: §import-copier, первый абзац).
  - Без `copier.yml` команда завершается ошибкой. Сторону проекта по `.copier-answers.yml` добавит следующий change.
- **Поддерживаемое подмножество** — ровно то, чем пользуется harness (ТЗ: §import-copier, «Template side»):
  - типы вопросов `bool`, `str`, `yaml`;
  - поля вопросов `choices`, `when`, `validator`, `default`, `help`;
  - ключи `_subdirectory`, `_templates_suffix`, `_envops`, `_exclude`, `_skip_if_exists`, `_tasks`, `_answers_file`, `_min_copier_version`.
- **Соответствие ключей** (ТЗ: §import-copier):

  | Copier | `rootstock.toml` |
  |---|---|
  | `_skip_if_exists` | `starter` |
  | `_exclude` вида `{% if _copier_operation == 'update' %}…{% endif %}` | отбрасывается: эти пути уже стартовые |
  | прочие `_exclude` | `exclude` |
  | `_tasks` | хуки с соответствующим `when` |

- **Шаблон файла ответов Copier** (`{{ _copier_conf.answers_file }}.jinja`) удаляется: его место занимает запись (ТЗ: §import-copier).
- **Отказ целиком.** Всё вне подмножества останавливает импорт со списком неподдерживаемого, и ничего не пишется (ТЗ: §import-copier). Сюда входят:
  - `_jinja_extensions` и `_migrations`;
  - неподдерживаемые типы вопросов;
  - прочие использования `_copier_conf` и `_copier_answers`.

  Половину шаблона конвертер не конвертирует никогда.

## Capabilities

### New Capabilities

- `copier-import`: команда `import-copier`, сторона шаблона. Описывает подмножество Copier, соответствие ключей, удаление шаблона файла ответов и отказ со списком неподдерживаемого.

### Modified Capabilities

Нет. Формат `rootstock.toml` не меняется: конвертер выдаёт то, что формат уже умеет.

## Impact

- **Код:**
  - новый модуль ядра `src/copier.rs` — чтение `copier.yml` и проверка дерева шаблона, без записи;
  - новая команда `src/commands/import_copier.rs` — запись `rootstock.toml` и удаление файла;
  - подкоманда в `src/cli.rs`.
- **Контракты** из таблицы «Источники правды» не меняются. Вывод конвертера проверяется существующей валидацией `rootstock.toml`.
- **Документация:**
  - `README.md`: быстрый старт и карта репозитория;
  - таблица модулей в `CLAUDE.md`.
- **Тесты:** unit-тесты `src/copier.rs`, сквозной `tests/cli_import_copier.rs` на фикстуре шаблона Copier и на `copier.yml` harness.

## Не входит

- Сторона проекта: чтение `.copier-answers.yml`, инвентарь и запись проекта — следующий change `stage2-import-copier-project`.
- Чтение `copier.yml` вместо `rootstock.toml` при `new` и `update`: это нужно стороне проекта, там и решается.
- Проверка паритета с Copier 9.19 на harness и screener (критерий приёмки этапа 2) — change `stage2-parity`. Здесь — только структура результата, а не байтовое совпадение рендера.
- Удаление `copier.yml` после импорта. ТЗ этого не требует; на этапе 3 мост harness ещё живёт на `copier.yml` (ТЗ: §Stages, п. 3).
- Возможности Copier за пределами подмножества: они не переводятся, а отклоняются.
