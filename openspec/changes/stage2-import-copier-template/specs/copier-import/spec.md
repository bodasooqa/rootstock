## ADDED Requirements

### Requirement: Команда import-copier на стороне шаблона

`rootstock import-copier`, запущенная в каталоге с `copier.yml`, SHALL читать его и писать рядом `rootstock.toml` с `schema = 1` (ТЗ: §import-copier, «Template side»). Запущенная в каталоге без `copier.yml` и без `copier.yaml`, команда MUST завершаться с кодом 1 (design D1).

Команда MUST завершаться с кодом 1, ничего не записав, в трёх случаях (design D1):
- `rootstock.toml` уже есть;
- есть оба файла, `copier.yml` и `copier.yaml`;
- `copier.yml` не разбирается как один YAML-документ.

Записанный `rootstock.toml` MUST проходить ту же проверку, что любой конфиг шаблона. Вывод, не прошедший её, — ошибка конвертера, а не файл на диске (design D2).

#### Scenario: Конвертация шаблона
- **WHEN** `import-copier` запущена в репозитории шаблона Copier, где все ключи из подмножества
- **THEN** рядом с `copier.yml` появляется `rootstock.toml`, код выхода 0, `copier.yml` остаётся на месте

#### Scenario: Конфиг уже есть
- **WHEN** в каталоге уже лежит `rootstock.toml`
- **THEN** код 1, файлы не изменены

#### Scenario: Нет copier.yml
- **WHEN** в каталоге нет ни `copier.yml`, ни `copier.yaml`
- **THEN** код 1, ничего не записано

### Requirement: Ключи шаблона Copier

Конвертер SHALL переносить ключи по ТЗ (§import-copier, «Template side»):
- `_subdirectory` → корень шаблона;
- `_templates_suffix` → суффикс шаблонизируемых файлов;
- `_envops` → настройки окружения Jinja `trim_blocks`, `lstrip_blocks`, `keep_trailing_newline`;
- `_skip_if_exists` → правила `starter`;
- `_exclude` → правила `exclude`;
- `_tasks` → хуки.

Шаблоны правил переносятся строками без изменений, вместе с Jinja (design D3).

Ключ, которого нет у Copier, конвертер MUST заменять тем, что делает Copier по умолчанию (design D4):
- без `_envops` `keep_trailing_newline` истинен, `trim_blocks` и `lstrip_blocks` ложны;
- без `_subdirectory` и без `_exclude` правила `exclude` получают список исключений Copier по умолчанию.

`_min_copier_version` и `_answers_file` SHALL отбрасываться (design D5).

#### Scenario: Ключи harness
- **WHEN** конвертируется `copier.yml` с `_subdirectory: template`, `_templates_suffix: .jinja` и тремя флагами `_envops`, равными true
- **THEN** в `rootstock.toml` корень `template`, суффикс `.jinja`, все три настройки Jinja истинны

#### Scenario: Правила с Jinja
- **WHEN** в `_skip_if_exists` есть `/README.md` и `{% if spec_path %}{{ spec_path }}/**{% endif %}`
- **THEN** оба шаблона стоят в `starter` дословно

#### Scenario: Исключения Copier по умолчанию
- **WHEN** в `copier.yml` нет ни `_subdirectory`, ни `_exclude`
- **THEN** `exclude` содержит исключения Copier по умолчанию, в том числе `copier.yml` и `.git`

### Requirement: Исключения только для обновления

Запись `_exclude` вида `{% if _copier_operation == 'update' %}P{% endif %}` SHALL отбрасываться, потому что путь `P` уже стартовый (ТЗ: §import-copier). Так же SHALL отбрасываться запись `{% if _copier_operation == 'update' and C %}P{% endif %}`, если в `_skip_if_exists` есть `{% if C %}P{% endif %}` (design D6).

Если в `_skip_if_exists` нет такой же записи, исключение не избыточно. Тогда импорт MUST останавливаться, и такое исключение попадает в список неподдерживаемого (design D6). Для первой формы такой записью считается `P`.

#### Scenario: Избыточное исключение
- **WHEN** в `_exclude` есть `{% if _copier_operation == 'update' %}/README.md{% endif %}`, а в `_skip_if_exists` — `/README.md`
- **THEN** в `exclude` его нет, `/README.md` стоит в `starter`

#### Scenario: Исключение с условием на ответ
- **WHEN** в `_exclude` есть `{% if _copier_operation == 'update' and spec_path %}{{ spec_path }}/**{% endif %}`, а в `_skip_if_exists` — `{% if spec_path %}{{ spec_path }}/**{% endif %}`
- **THEN** исключение отброшено

#### Scenario: Исключение без пары
- **WHEN** в `_exclude` есть `{% if _copier_operation == 'update' %}/a.txt{% endif %}`, а `/a.txt` нет в `_skip_if_exists`
- **THEN** код 1, ничего не записано, исключение названо в списке неподдерживаемого

### Requirement: Задачи Copier в хуки

Каждая задача `_tasks` SHALL становиться хуком с той же командой и в том же порядке (ТЗ: §import-copier). Фаза хука зависит от условия задачи (design D7):

| Условие задачи | Фаза хука |
|---|---|
| `{{ _copier_operation == 'copy' }}` | только `new` |
| `{{ _copier_operation == 'update' }}` | только `update` |
| нет условия или `true` | обе фазы |

Задача-строка SHALL считаться задачей без условия.

Задача MUST попадать в список неподдерживаемого в трёх случаях (design D7):
- у неё другое условие;
- её команда — список или содержит Jinja (`{{` или `{%`);
- у неё есть ключи кроме `command` и `when`.

#### Scenario: Задача только при создании
- **WHEN** задача `{command: "openspec init", when: "{{ _copier_operation == 'copy' }}"}`
- **THEN** в `rootstock.toml` хук `openspec init` с фазой `new`

#### Scenario: Команда с Jinja
- **WHEN** команда задачи содержит `{{ project_name }}`
- **THEN** код 1, задача названа в списке неподдерживаемого

### Requirement: Вопросы Copier

Вопросы SHALL переноситься в порядке `copier.yml`, с теми же именами, `help`, `default` и `validator` (ТЗ: §import-copier). Типы `bool`, `str` и `yaml` SHALL переноситься как есть. `str` с `choices` SHALL становиться вопросом `choice` (design D8):
- у словаря `choices` ключ — подпись, значение — значение;
- элемент списка `choices` — значение и подпись сразу.

`when` переносится по виду (design D8):
- `"{{ E }}"` — выражение `E`;
- `true` или `false` — то же значение.

Значение `default` у `yaml` SHALL переноситься структурой.

Вопрос MUST попадать в список неподдерживаемого в любом из случаев (ТЗ: §import-copier; design D8):
- у него нет `type`;
- тип не из подмножества;
- есть поле кроме `type`, `help`, `default`, `when`, `validator`, `choices`;
- `when` другого вида;
- `choices` не у `str`;
- значение в `choices` — не скаляр;
- в значении есть `null`.

#### Scenario: Вопрос с выбором
- **WHEN** вопрос `doc_form: {type: str, default: catalog, choices: {"catalog — каталогом": catalog, "monolith — одним файлом": monolith}}`
- **THEN** в `rootstock.toml` вопрос `choice` с двумя вариантами `{label, value}` и дефолтом `catalog`

#### Scenario: Условие вопроса
- **WHEN** у вопроса `when: "{{ not (rust and frontend) }}"`
- **THEN** у вопроса в `rootstock.toml` `when = "not (rust and frontend)"`

#### Scenario: Словарь по умолчанию
- **WHEN** у вопроса `yaml` дефолт — словарь из трёх строк
- **THEN** дефолт в `rootstock.toml` — таблица с теми же ключами и значениями

#### Scenario: Неподдерживаемый тип
- **WHEN** у вопроса `type: int`
- **THEN** код 1, вопрос назван в списке неподдерживаемого

### Requirement: Шаблон файла ответов Copier

Файл в корне шаблона с именем `{{ _copier_conf.answers_file }}` плюс суффикс шаблонизируемых файлов SHALL удаляться вместе с записью `rootstock.toml`: его место занимает запись проекта (ТЗ: §import-copier). Пробелы внутри `{{ }}` в имени MAY быть любыми (design D9).

#### Scenario: Файл ответов удалён
- **WHEN** в `template/` лежит `{{ _copier_conf.answers_file }}.jinja`
- **THEN** после импорта его нет, а `new` по сконвертированному шаблону не выпускает `.copier-answers.yml`

### Requirement: Отказ со списком неподдерживаемого

Импорт MUST не писать ничего и завершаться с кодом 1, если находит хоть что-то вне подмножества (ТЗ: §import-copier). Список неподдерживаемого MUST идти в stderr целиком, по строке на находку, а не только первая (design D10). В список попадают:
- ключи с `_` вне подмножества (`_jinja_extensions`, `_migrations`, `_preserve_symlinks` и любые другие);
- ключи `_envops` кроме трёх поддержанных и значения, отличные от true и false;
- Jinja в `_subdirectory` или `_templates_suffix`, пустой `_templates_suffix`;
- YAML-теги;
- использования переменных Copier (`_copier_conf`, `_copier_answers`, `_copier_operation` и других `_copier_*`, `_folder_name`, `_external_data`) вне разобранных выше форм: в именах и текстовых файлах под корнем шаблона, в вопросах и в правилах (design D11);
- символические ссылки под корнем шаблона: Copier по умолчанию их разыменовывает, а rootstock воспроизводит ссылкой (design D12);
- неподдерживаемые вопросы, задачи и исключения из требований выше.

#### Scenario: Несколько находок
- **WHEN** в `copier.yml` есть `_jinja_extensions` и вопрос `type: int`, а в шаблоне файл использует `{{ _copier_conf.src_path }}`
- **THEN** код 1, `rootstock.toml` не создан, в stderr названы все три находки

#### Scenario: Символическая ссылка
- **WHEN** под корнем шаблона есть символическая ссылка
- **THEN** код 1, ссылка названа в списке неподдерживаемого

#### Scenario: harness конвертируется без находок
- **WHEN** `import-copier` запущена на копии репозитория harness на последнем теге
- **THEN** код 0, `rootstock.toml` проходит проверку конфига, шаблон файла ответов удалён
