## Why

Этап 2 — «Copier import and parity». Ревизия ТЗ 1.2 добавила в него Python-методы на значениях (ТЗ: §Stages, п. 2; §Template format, «Filters and tests»).

Шаблоны harness во всех тегах, от v0.3.3 до v0.4.5, вызывают два метода, которых нет в minijinja:
- `names.append(name)` в `template/_partials/agents/meta.jinja`;
- `open_questions_path.endswith('.md')` в `knowledge/README.md.jinja`.

Поэтому сконвертированный harness не рендерится вовсе. Сторона проекта `import-copier` и проверка паритета рендерят именно эти теги, так что change идёт перед ними.

## What Changes

- **Методы строк, словарей и списков** с семантикой Python (ТЗ: §Template format, 1.2):
  - строк: `startswith`, `endswith`, `lower`, `upper`, `strip`, `split`, `replace` и другие распространённые;
  - словарей: `items`, `keys`, `values`, `get`.

  Метод вне набора, как и прежде, — ошибка рендера.
- **`list.append(x)`** меняет список на месте, как в Python (ТЗ: §Template format, 1.2). Метод работает у списка, заведённого `{% set x = [] %}`. У прочих списков он — ошибка рендера с понятным сообщением (design D3).
- Методы действуют везде, где rootstock рендерит Jinja:
  - содержимое и имена файлов;
  - подключаемые файлы;
  - `default` и `validator` вопросов;
  - правила классов файлов.

## Capabilities

### New Capabilities

Нет.

### Modified Capabilities

- `template-format`: требование «Настройки окружения Jinja и подключения» — к фильтрам и тестам добавляются Python-методы на значениях.

## Impact

- **Код:** `src/jinja.rs` — колбэк неизвестных методов, изменяемый список, подмена литерала `[]` при загрузке исходника.
- **Зависимость:** `minijinja-contrib` с возможностью `pycompat`, той же версии, что `minijinja`. Список библиотек в `CLAUDE.md` дополняется.
- **Контракты** из таблицы «Источники правды» не меняются. `Cargo.lock` обновляет cargo.
- **Документация:** раздел формата шаблона в `README.md`.
- **Тесты:** unit-тесты `src/jinja.rs`, сквозной рендер фрагментов harness.

## Не входит

- Прочие отличия minijinja от Jinja2. Если они всплывут, ими займётся change `stage2-parity`, сверяя рендер с Copier 9.19.
- Изменяемость списков, созданных иначе чем `{% set x = [] %}`: литерал с элементами, атрибуты `namespace`, аргументы макросов. ТЗ требует `append` в том виде, в каком им пользуется harness.
- Мутирующие методы, кроме `append` (`extend`, `pop`, `update` и другие).
- Сторона проекта `import-copier` и паритет — следующие changes этапа.
