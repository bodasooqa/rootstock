## 1. Слияние через git merge-file

- [x] 1.1 Написать падающие тесты `GitMerge` в `src/apply.rs`. Старые случаи из `src/merge.rs` переносятся с теми же ожиданиями:
  - пересечение;
  - пустая база;
  - CRLF;
  - нет завершающего перевода строки;
  - CRLF при неполной стороне;
  - похожий на маркер текст.

  Добавить векторы, снятые с `git merge-file` (design D1):
  - одинаковая правка на краю конфликта;
  - склейка конфликтов через строку `}`;
  - два конфликта через четыре строки с буквами;
  - два случая, где git сливает чисто, а diffy давал конфликт.

  Добавить короткие пути D3 и отказ git: несуществующий исполняемый файл даёт ошибку в `finish()`. Тесты новых векторов красные на старом `merge3`.
- [x] 1.2 Реализовать трейт `Merge3` в `src/merge.rs` и `GitMerge` в `src/apply.rs` по design D1–D5. Убрать diffy-реализацию, `safe_marker_len`, `relabel` и зависимость `diffy` из `Cargo.toml`. Проверка: тесты 1.1 зелёные, `cargo tree | grep diffy` пуст.
- [x] 1.3 Передать слияние в `plan_update` параметром `&dyn Merge3`. В `new`, `update` и `import-copier` вызвать `finish()` до первой записи. Тесты `src/plan.rs` идут через `GitMerge`. Проверка: `cargo test` зелёный.
- [x] 1.4 Сквозной тест в `tests/cli_new_update.rs`: `update` с `GIT_CONFIG_GLOBAL`, где задано `merge.conflictStyle = diff3`, даёт конфликт без строки `|||||||`, код 2. Красный до 1.3 проверить нельзя: diffy конфиг git не читает. Поэтому тест проверяется подменой `-c` в коде на время прогона.

## 2. Сверка с Copier

- [x] 2.1 Скрипты `tests/parity/render.sh`, `tests/parity/update.sh` и `tests/parity/README.md` по design D6. Проверка: `render.sh` на v0.4.5 — 64 комбинации без расхождений; `update.sh` на screener `f274eefc` → v0.4.5 — без расхождений после нормализации подписей маркеров.

## 3. Документы и проверки

- [x] 3.1 `README.md`: слияние идёт через `git merge-file`, git нужен в PATH. `CLAUDE.md`: из стека убран diffy, у `apply.rs` в таблице появляется `GitMerge`. Проверка: `grep -n diffy README.md CLAUDE.md` пуст.
- [x] 3.2 Полные проверки: `cargo fmt --check`, `cargo clippy --all-targets --all-features -- -D warnings`, `cargo test`.
