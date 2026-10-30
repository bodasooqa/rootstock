## Context

Мотивация — `proposal.md`. ТЗ: §import-copier («Project side», «Old template tags never need converting»), §How update works (правила 1, 11), §Non-goals.

Текущее состояние кода:
- `src/copier.rs::convert(yaml, template_dir)` даёт `Converted { text, config, remove }`. `config` уже прошёл `config::parse`, а `remove` — пути шаблона файла ответов Copier относительно каталога шаблона;
- `src/commands/import_copier.rs::run(dir, err)` — только сторона шаблона;
- `Side::load(checkout)` в `src/commands/mod.rs` читает только `rootstock.toml`;
- `update` грузит обе стороны через `Side::load`;
- `require_clean_tree` живёт в `src/commands/update.rs`.

Живые данные:
- `.copier-answers.yml` у screener и у самого rootstock: `_commit: v0.4.5`, `_src_path: gh:bodasooqa/harness`, плоские ответы. Значения — скаляры и словарь (`pi_models`). Ответа на `monorepo` нет: его `when` ложен;
- шаблон файла ответов harness — `template/{{ _copier_conf.answers_file }}.jinja` с фильтром `to_nice_yaml`, которого в rootstock нет.

Затронутые модули:
- ядро — `copier` (разбор файла ответов), `config` (поле пропуска), `render` (пропуск исходников);
- края — `commands`.

Граф модулей не меняется. Контракты «Источников правды» не меняются: поле пропуска в `TemplateConfig` не читается из `rootstock.toml` и не пишется в него.

## Goals / Non-Goals

**Goals:**
- `import-copier` в копии screener и в копии rootstock даёт рабочую запись на v0.4.5;
- `update` такого проекта на тег в формате rootstock проходит без ручной конвертации старого тега.

**Non-Goals:**
- паритет рендера с Copier — это `stage2-parity`;
- `new` из шаблона в формате Copier;
- нестандартное имя файла ответов.

## Decisions

**D1. Сторона выбирается по файлам в каталоге.** Порядок проверки:
1. есть `copier.yml` или `copier.yaml` и нет `.copier-answers.yml` — сторона шаблона, прежние проверки без изменений;
2. нет файла шаблона, есть `.copier-answers.yml` — сторона проекта;
3. есть и то, и другое — ошибка: «both copier.yml and .copier-answers.yml are here: this is a template and a project at once; nothing was changed»;
4. нет ничего — ошибка, названы все три имени.

Наличие проверяется через `symlink_metadata`, как сейчас. Ссылка вместо файла — ошибка «must be a regular file».
- *Почему.* ТЗ различает стороны только по тому, где запущена команда, и флага не вводит. Каталог, который одновременно и шаблон, и проект, бывает: шаблон сам может быть сгенерирован шаблоном. Угадывать сторону в таком каталоге нельзя.
- *Отвергнуто.* Флаг `--project` / `--template`: в ТЗ его нет.

**D2. Сторона проекта собирает всё до первой записи.** Шаги (правило 11 ТЗ):
1. Проверки до сети:
   - `.rootstock.toml` нет;
   - дерево чистое (D4);
   - файл ответов разобран (D3).
2. `parse_source(_src_path)`. Локальный относительный путь берётся от каталога проекта, а в запись идёт абсолютным, как у `new` (`recorded_source`). `gh:` и URL записываются как есть.
3. `check_version(_commit)` без `HEAD`, затем `checkout(src, None, _commit)` и `Side::load_any` (D5).
4. Лишние ключи ответов (D3). Затем `side.collect(answers, use_defaults = true, NoPrompt)` — тот же путь, что у базы в `update`.
5. Рендер с `collected.context`. Инвентарь — `plan_update(пусто, рендер, DiskProject, пустой инвентарь).inventory`, как у `new`: все выданные пути, кроме `blocked`. Действия плана не выполняются, `project.finish()` проверяется. Из конфликтов плана сохраняются только `blocked`: их путь rootstock вести не будет, а это требует человека (правило 11 ТЗ). Остальные конфликты пустой базы (`added-by-both`) — это правки проекта, о них импорт молчит.
6. `record::to_text(Record { schema: 1, source, subdir: None, version: _commit, commit: checkout.commit, answers: collected.recorded, inventory })`.
7. Запись `.rootstock.toml` (`record::write_text`), затем удаление `.copier-answers.yml`. Ошибка удаления: «.rootstock.toml is already written, but cannot remove .copier-answers.yml». В stderr — `wrote .rootstock.toml` и `removed .copier-answers.yml`.
8. Конфликты `blocked` печатаются в stdout строками `blocked <path>`, как текстовый список `new`, и дают код 2. `import_copier::run` возвращает их списком, у стороны шаблона список пуст.

Ошибки шагов 3–5 несут источник: `_commit` и `.copier-answers.yml` у проверки версии и ответов, версию у рендера. Проверки «файла нет» различают `NotFound` и прочие ошибки: прочие — ошибка, а не «нет файла». Так сделано и для `rootstock.toml` в `load_any`.

Хуки не запускаются, файлы проекта не трогаются.
- *Почему.* ТЗ: «The project is now a rootstock project at its old version». Проект уже содержит рендер этой версии, сделанный Copier'ом. Запускать задачи `copy` повторно — значит менять проект, о чём импорт не просил.
- *Цена.* Если рендер rootstock отличается от рендера Copier, это проявится только на первом `update`: managed-файл с отличием даёт трёхстороннее слияние, а не тихую перезапись. Сами отличия — предмет `stage2-parity`.

**D3. Разбор файла ответов — в ядре, `copier::parse_answers(yaml) -> Result<CopierAnswers, CopierError>`.**
- Результат: `CopierAnswers { src_path: String, commit: String, answers: Answers }`, где `Answers` — тот же `BTreeMap<String, serde_json::Value>`, что у записи.
- Документ MUST быть YAML-словарём со строковыми ключами. `_src_path` и `_commit` обязательны и должны быть строками.
- Прочие ключи на `_` — ошибка: Copier пишет только эти два.
- Значения переводятся из YAML в JSON. Значение, которое нельзя записать в TOML, — ошибка с именем ключа: `null`, целое вне `i64`, `NaN`, теги.

Сверка с вопросами — в команде, после конвертации:
- ключ, которого нет среди `cfg.questions`, — ошибка «.copier-answers.yml: `<key>` is not a question of the template at <version>»;
- тип и валидатор проверяет `collect`;
- нет ответа — берётся значение по умолчанию (`use_defaults = true`), без значения по умолчанию — ошибка `NoPrompt`.
- *Почему.* Тот же путь, по которому `update` читает ответы базы, поэтому импорт и первое обновление понимают ответы одинаково. Ответ на вопрос с ложным `when` Copier не пишет, а `collect` его и не ждёт.
- *Отвергнуто.* Молча отбрасывать лишние ключи: правленый руками файл ответов дал бы проект с неверными ответами без единого слова.

**D4. Сторона проекта требует чистого git-дерева.** `require_clean_tree` переезжает из `update.rs` в `commands/mod.rs` и зовётся до чтения файла ответов, но после проверки `.rootstock.toml`.
- *Почему.* ТЗ, §How update works, правило 1: каждое изменение проекта rootstock'ом должно просматриваться и откатываться git'ом. Импорт пишет запись и удаляет файл. *(Решение исполнителя: ТЗ для импорта этого не оговаривает.)*
- *Последствие.* `update` сразу после импорта откажет, пока импорт не закоммичен: дерево грязное. Мост в `harness-update.yml` (ТЗ: §Repository and release) коммитит импорт отдельным коммитом перед `update`. Это работа репозитория harness, записывается в отчёт.

**D5. Конвертация в памяти — `Side::load_any`, только для `update` и импорта проекта.**
- `Side::load(checkout)`: прежнее поведение. Без `rootstock.toml` и при наличии `copier.yml` или `copier.yaml` ошибка получает подсказку: «template version <commit> has copier.yml but no rootstock.toml; run `rootstock import-copier` in the template repository». Так работает `new`.
- `Side::load_any(checkout)`:
  - есть `rootstock.toml` — он;
  - иначе есть ровно один из `copier.yml` / `copier.yaml`, обычный файл — `copier::convert(yaml, checkout.path())`;
  - находки печатаются в stderr тем же форматом, что на стороне шаблона, и дают код 1;
  - конфиг — `converted.config` с пропуском `converted.remove`.

  Так грузятся обе стороны `update` и сторона проекта `import-copier`.
- *Почему.* ТЗ: «Old template tags never need converting» и «converts its copier.yml in memory (same rules as the template side)». Критерий приёмки этапа обновляет screener с тега в формате Copier. Для `new` ТЗ в §Non-goals запуск шаблонов Copier без конвертации отвергает. Новый проект всегда можно создать с версии, сконвертированной заранее.
- *Отвергнуто.* Конвертация в `Side::load` для всех команд: `new` стал бы молча принимать шаблоны Copier.

**D6. Пропуск исходников — поле `TemplateConfig::skip`, `Vec<RelPath>`, `#[serde(skip)]`.**
- Пути — относительно каталога шаблона, в той же форме, что `Converted::remove`. `copier::convert` кладёт в поле свой `remove`.
- `render_tree` не обходит исходный файл, чей путь от каталога шаблона совпадает с элементом `skip`: ни имя, ни содержимое не рендерятся.
- Из `rootstock.toml` поле не читается (`deny_unknown_fields` по-прежнему отвергает такой ключ) и в него не пишется.
- *Почему.* Шаблон файла ответов harness рендерит `to_nice_yaml`, которого в rootstock нет. Удалить файл из checkout'а нельзя: `checkout` локального `HEAD` — рабочее дерево шаблона.
- *Отвергнуто.* Правило `exclude`: оно сверяется с отрендеренным путём, а имя `{{ _copier_conf.answers_file }}` рендерится в пустую строку, ещё не дойдя до правил.

**D7. Готовый файл ответов в корне шаблона исключается.** Если конвертер нашёл шаблон файла ответов, он добавляет в `files.exclude` правило `/<_answers_file>`, по умолчанию `/.copier-answers.yml`. Правило попадает и в записанный `rootstock.toml` стороны шаблона. Имя попадает в правило как glob, поэтому `_answers_file` вне набора `A–Z a–z 0–9 . _ - /` даёт находку, как и любой компонент `..`: и Jinja в имени, и символы glob (`*`, `?`, `[`, `!`, `#`), и пробелы.
- *Почему.* Шаблон, который сам сгенерирован Copier'ом и не использует `_subdirectory`, несёт в корне свой `.copier-answers.yml`. Без правила рендер выдал бы его managed-файлом: импорт записал бы его в инвентарь и тут же удалил. Каждый следующий `update` давал бы `deleted-by-project`. Нашёл rust-reviewer, воспроизведено.
- *Отвергнуто.* Отдельное исключение только на стороне проекта: сконвертированный на диске шаблон выдал бы тот же файл в `new`.

## Risks / Trade-offs

- [`_src_path: gh:…` требует сети на импорте и на каждом `update`] → так же, как у проекта, созданного `new` из `gh:`. Живая проверка подменяет источник на локальный клон в копии проекта.
- [Ответы, записанные Copier'ом, расходятся с типами вопроса rootstock, например `int` строкой] → `collect` даёт ошибку с именем вопроса, импорт ничего не пишет. Для harness типы сверены живой проверкой.
- [Мост коммитит импорт отдельно (D4)] → записывается в отчёт по этапу как требование к `harness-update.yml`.
