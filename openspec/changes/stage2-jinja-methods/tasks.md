## 1. Не-мутирующие методы (design D1)

- [ ] 1.1 Падающие тесты в `src/jinja.rs`:
  - `{{ p[:-3] if p.endswith('.md') else p }}` при `p = "k/oq.md"` → `k/oq`;
  - `{{ "Ab".lower() }}`, `{{ d.items() | list }}`, `{{ d.get("x", 1) }}`;
  - метод вне набора — ошибка рендера.

  Проверка: `cargo test jinja` красный ровно на новых тестах.
- [ ] 1.2 Добавить `minijinja-contrib = { version = "3.0.4", features = ["pycompat"] }` (`cargo add`). В `Jinja::new` поставить `set_unknown_method_callback`. Проверка: `cargo test jinja` зелёный.

## 2. Изменяемый список и подмена литерала (design D2, D3, D4)

- [ ] 2.1 Падающие тесты:
  - макрос harness `pi_tools` с вложенными циклами и `names.append(name)` даёт `read, find, ls`; `length` и `count` верны;
  - `append` у `{% set x = [1] %}` — ошибка с подсказкой из D3;
  - `{%- set x = [] -%}` и `{% set x=[] %}` подменяются, результат при `trim_blocks`/`lstrip_blocks` побайтно тот же, что у `[]`;
  - текст `{% set x = [] %}` внутри `{% raw %}`, в комментарии `{# #}` и в строке внутри `{{ "…" }}` не меняется;
  - `append` работает в подключаемом файле (`include` из каталога шаблона) и в имени файла через `render_str`;
  - `rewrite_list_literals` на тексте без такой формы возвращает его байт в байт.

  Проверка: красные.
- [ ] 2.2 Реализовать:
  - `MutableList`;
  - глобальную функцию `__rootstock_list`;
  - `rewrite_list_literals` в `render_str` и в загрузчике;
  - подсказку для `append` у других списков.

  Проверка: `cargo test` зелёный.

## 3. Документация

- [ ] 3.1 Обновить документы:
  - `README.md`, раздел «Template format»: Python-методы на значениях и правило для `append`;
  - `CLAUDE.md`: `minijinja-contrib` в списке библиотек.

## 4. Проверки

- [ ] 4.1 `cargo fmt --check`, `cargo clippy --all-targets --all-features -- -D warnings`, `cargo test` — зелёные.
- [ ] 4.2 Живая проверка в scratchpad: harness v0.4.5 и v0.3.3, сконвертированные `import-copier`, проходят `new` при всех допустимых сочетаниях `rust`, `frontend`, `doc_form`.
