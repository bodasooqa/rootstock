# copier-import Specification

## Purpose
Одноразовый перевод шаблонов и проектов Copier на rootstock (ТЗ: §import-copier). Сторона шаблона читает `copier.yml` и пишет `rootstock.toml`. Сторона проекта читает `.copier-answers.yml` и пишет `.rootstock.toml` на той же версии. `update` конвертирует старые теги в формате Copier в памяти. Всё вне поддерживаемого подмножества останавливает импорт со списком неподдерживаемого.

## Requirements

### Requirement: Команда import-copier на стороне шаблона

`rootstock import-copier`, запущенная в каталоге с `copier.yml`, SHALL читать его и писать рядом `rootstock.toml` с `schema = 1` (ТЗ: §import-copier, «Template side»). Сторона выбирается по содержимому каталога (design D1):
- есть `copier.yml` или `copier.yaml` — сторона шаблона;
- их нет, но есть `.copier-answers.yml` — сторона проекта;
- нет ни того, ни другого — код 1, ничего не записано;
- есть и файл шаблона, и `.copier-answers.yml` — обе стороны за один запуск (требование «Шаблон, который сам проект Copier'а»).

Команда на стороне шаблона MUST завершаться с кодом 1, ничего не записав, в трёх случаях (design D1 change'а `stage2-import-copier-template`):
- `rootstock.toml` уже есть;
- есть оба файла, `copier.yml` и `copier.yaml`;
- `copier.yml` не разбирается как один YAML-документ.

Записанный `rootstock.toml` MUST проходить ту же проверку, что любой конфиг шаблона. Вывод, не прошедший её, — ошибка конвертера, а не файл на диске (design D2 change'а `stage2-import-copier-template`).

#### Scenario: Конвертация шаблона
- **WHEN** `import-copier` запущена в репозитории шаблона Copier, где все ключи из подмножества
- **THEN** рядом с `copier.yml` появляется `rootstock.toml`, код выхода 0, `copier.yml` остаётся на месте

#### Scenario: Конфиг уже есть
- **WHEN** в каталоге уже лежит `rootstock.toml`
- **THEN** код 1, файлы не изменены

#### Scenario: Нет copier.yml
- **WHEN** в каталоге нет ни `copier.yml`, ни `copier.yaml`, ни `.copier-answers.yml`
- **THEN** код 1, ничего не записано

#### Scenario: Шаблон и проект сразу
- **WHEN** в каталоге лежат и `copier.yml`, и `.copier-answers.yml`
- **THEN** конвертируются обе стороны за один запуск, по требованию «Шаблон, который сам проект Copier'а»

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

Файл в корне шаблона с именем `{{ _copier_conf.answers_file }}` плюс суффикс шаблонизируемых файлов SHALL удаляться вместе с записью `rootstock.toml`: его место занимает запись проекта (ТЗ: §import-copier). Пробелы внутри `{{ }}` в имени MAY быть любыми (design D9 change'а `stage2-import-copier-template`).

Когда такой файл найден, в `exclude` SHALL добавляться правило `/<имя файла ответов>`. Имя берётся из `_answers_file`, по умолчанию `.copier-answers.yml`. Так файл ответов, который лежит в корне шаблона готовым файлом, тоже не попадает в проект (design D7).

#### Scenario: Файл ответов удалён
- **WHEN** в `template/` лежит `{{ _copier_conf.answers_file }}.jinja`
- **THEN** после импорта его нет, а `new` по сконвертированному шаблону не выпускает `.copier-answers.yml`

#### Scenario: Готовый файл ответов в корне шаблона
- **WHEN** корень шаблона — каталог с `copier.yml`, и рядом с `{{ _copier_conf.answers_file }}.jinja` лежит готовый `.copier-answers.yml`
- **THEN** `rootstock.toml` исключает `/.copier-answers.yml`, рендер его не выдаёт, инвентарь импортированного проекта его не содержит

### Requirement: Отказ со списком неподдерживаемого

Импорт MUST не писать ничего и завершаться с кодом 1, если находит хоть что-то вне подмножества (ТЗ: §import-copier). Список неподдерживаемого MUST идти в stderr целиком, по строке на находку, а не только первая (design D10). В список попадают:
- ключи с `_` вне подмножества (`_jinja_extensions`, `_migrations`, `_preserve_symlinks` и любые другие);
- ключи `_envops` кроме трёх поддержанных и значения, отличные от true и false;
- Jinja в `_subdirectory` или `_templates_suffix`, пустой `_templates_suffix`;
- YAML-теги;
- использования переменных Copier (`_copier_conf`, `_copier_answers`, `_copier_operation` и других `_copier_*`, `_folder_name`, `_external_data`) вне разобранных выше форм: в именах и текстовых файлах под корнем шаблона, в вопросах и в правилах (design D11);
- символические ссылки под корнем шаблона: Copier по умолчанию их разыменовывает, а rootstock воспроизводит ссылкой (design D12);
- `_subdirectory`, который выходит за каталог шаблона, абсолютен, проходит через символическую ссылку, не существует или не каталог. Под таким корнем ничего не читается и не удаляется (design D14);
- имена не в UTF-8, FIFO, сокеты и устройства под корнем шаблона (design D15);
- неподдерживаемые вопросы, задачи и исключения из требований выше.

#### Scenario: Несколько находок
- **WHEN** в `copier.yml` есть `_jinja_extensions` и вопрос `type: int`, а в шаблоне файл использует `{{ _copier_conf.src_path }}`
- **THEN** код 1, `rootstock.toml` не создан, в stderr названы все три находки

#### Scenario: Символическая ссылка
- **WHEN** под корнем шаблона есть символическая ссылка
- **THEN** код 1, ссылка названа в списке неподдерживаемого

#### Scenario: Корень за пределами шаблона
- **WHEN** `_subdirectory: ../outside` или `_subdirectory: a/b`, где `a` — символическая ссылка
- **THEN** код 1, `_subdirectory` назван в списке неподдерживаемого, файлы вне каталога шаблона не прочитаны и не удалены

#### Scenario: harness конвертируется без находок
- **WHEN** `import-copier` запущена на копии репозитория harness на последнем теге
- **THEN** код 0, `rootstock.toml` проходит проверку конфига, шаблон файла ответов удалён

### Requirement: Команда import-copier на стороне проекта

`rootstock import-copier`, запущенная в проекте с `.copier-answers.yml`, SHALL перевести проект на rootstock на его текущей версии шаблона (ТЗ: §import-copier, «Project side»):
- источник — `_src_path`, версия — `_commit`, ответы — остальные ключи файла;
- шаблон берётся на теге `_commit`, его `copier.yml` конвертируется в памяти по правилам стороны шаблона; если на этом теге уже есть `rootstock.toml`, берётся он, как у `update`;
- шаблон рендерится с записанными ответами, инвентарь строится по рендеру так же, как у `new`;
- пишется `.rootstock.toml`: тот же источник, версия `_commit`, коммит этого тега, ответы, инвентарь, `schema = 1`;
- после записи удаляется `.copier-answers.yml`.

Прочие файлы проекта MUST остаться нетронутыми, хуки MUST не запускаться (design D2).

Путь, который шаблон выдаёт, а проект занял тем, что держит (правило 12 ТЗ), не попадает в инвентарь. Такой путь MUST печататься как конфликт `blocked` в stdout, а команда MUST завершаться с кодом 2 после записи (ТЗ: §How update works, правила 11 и 12; design D2). Прочие расхождения проекта с рендером — правки проекта, о них импорт не сообщает.

Команда MUST завершаться с кодом 1, ничего не записав и не удалив, в таких случаях (design D3, D4):
- `.rootstock.toml` уже есть;
- `.copier-answers.yml` — не обычный файл или не разбирается как YAML-словарь;
- нет `_src_path` или `_commit`;
- `_commit` не тег версии (`v` и semver, как у `new --version`) или такого тега в шаблоне нет;
- в файле есть ключ, который не является вопросом шаблона этой версии и не входит в `_src_path`, `_commit`;
- ответ не проходит проверку типа или валидатор вопроса;
- рабочее дерево проекта не git или в нём есть незакоммиченные изменения;
- шаблон этой версии нельзя сконвертировать — тогда печатается тот же список неподдерживаемого, что на стороне шаблона.

Ответ на вопрос, которого нет в файле, SHALL браться из значения по умолчанию так же, как у базы при `update` (design D3).

#### Scenario: Проект переведён на rootstock
- **WHEN** `import-copier` запущена в чистом проекте Copier, чей `_commit` — тег шаблона в поддерживаемом подмножестве
- **THEN** код 0, появляется `.rootstock.toml` с источником `_src_path`, версией `_commit`, коммитом этого тега и ответами файла, `.copier-answers.yml` удалён, остальные файлы не изменены

#### Scenario: Инвентарь по рендеру
- **WHEN** шаблон на теге `_commit` с записанными ответами выдаёт управляемый файл и стартовый файл, а проект стартовый файл удалил
- **THEN** оба пути в инвентаре со своими классами, и следующий `update` стартовый файл не создаёт заново

#### Scenario: Версия не тег
- **WHEN** `_commit` — `v0.4.5-3-gabc1234`
- **THEN** код 1, ничего не записано и не удалено

#### Scenario: Лишний ключ в ответах
- **WHEN** в `.copier-answers.yml` есть ключ, которого нет среди вопросов шаблона этой версии
- **THEN** код 1, сообщение называет ключ, ничего не записано

#### Scenario: Незакоммиченные изменения
- **WHEN** в рабочем дереве проекта есть незакоммиченные изменения
- **THEN** код 1, ничего не записано и не удалено

#### Scenario: Запись уже есть
- **WHEN** в проекте уже есть `.rootstock.toml`
- **THEN** код 1, файлы не изменены

#### Scenario: Обновление после импорта
- **WHEN** проект переведён `import-copier`, изменения закоммичены, проект правил managed-файл, а следующий тег шаблона меняет в нём другие строки
- **THEN** `rootstock update` сливает файл трёхсторонне от записанного тега: в файле и правка проекта, и правка шаблона, код 0

#### Scenario: Путь занят проектом
- **WHEN** шаблон на теге `_commit` выдаёт файл `a/b`, а в проекте `a` — обычный файл
- **THEN** запись написана без `a/b` в инвентаре, в stdout `blocked a/b`, код 2

#### Scenario: rootstock.toml на теге проекта
- **WHEN** на теге `_commit` лежат и `copier.yml`, и `rootstock.toml`, и они расходятся в классе файла
- **THEN** инвентарь строится по `rootstock.toml`

### Requirement: Обновление с версии шаблона в формате Copier

`rootstock update` SHALL принимать версию шаблона, в которой нет `rootstock.toml`, но есть `copier.yml` или `copier.yaml`: и как базу, и как цель. Такая версия конвертируется в памяти по правилам стороны шаблона (ТЗ: §import-copier, «Old template tags never need converting»; design D5). Ничего из этого не пишется ни в шаблон, ни в проект.

- Если в версии есть `rootstock.toml`, берётся он, даже когда рядом лежит `copier.yml`.
- Шаблон файла ответов Copier, который убрал бы конвертер на стороне шаблона, в рендер не попадает.
- Версия, которую нельзя сконвертировать, MUST давать код 1 со списком неподдерживаемого, ничего не записав.
- `new` такую версию MUST не принимать: код 1 и подсказка запустить `import-copier` в шаблоне (ТЗ: §Non-goals, «Running Copier templates unconverted»; design D5).

#### Scenario: База в формате Copier, цель в формате rootstock
- **WHEN** проект записан на теге, где есть только `copier.yml`, а целевой тег несёт `rootstock.toml`
- **THEN** база рендерится из конвертированного в памяти `copier.yml`, цель — из `rootstock.toml`, обновление идёт по таблице

#### Scenario: Обе версии в формате Copier
- **WHEN** и записанный, и целевой тег несут только `copier.yml`
- **THEN** обе стороны конвертируются в памяти, обновление идёт по таблице

#### Scenario: Файл ответов Copier не возвращается
- **WHEN** база или цель в формате Copier содержит шаблон `{{ _copier_conf.answers_file }}.jinja`
- **THEN** `.copier-answers.yml` не появляется в проекте и не попадает в инвентарь

#### Scenario: Неподдерживаемое в старом теге
- **WHEN** целевой тег в формате Copier использует `_migrations`
- **THEN** код 1, список неподдерживаемого в stderr, ничего не записано

#### Scenario: new из шаблона Copier
- **WHEN** `new` запущена на теге, где есть только `copier.yml`
- **THEN** код 1, сообщение советует `import-copier`, ничего не записано

### Requirement: Шаблон, который сам проект Copier'а

Если в каталоге лежат и `copier.yml` (или `copier.yaml`), и `.copier-answers.yml`, `rootstock import-copier` SHALL конвертировать обе стороны за один запуск (ТЗ: §import-copier, 1.3). Сторона шаблона пишет `rootstock.toml` по своим правилам, сторона проекта пишет `.rootstock.toml` и удаляет `.copier-answers.yml` по своим.

Все проверки обеих сторон MUST проходить до первой записи. Сюда входят:
- чистое дерево;
- уже существующие `rootstock.toml` и `.rootstock.toml`;
- неподдерживаемое в `copier.yml`;
- `_commit`, который не является тегом;
- выгрузка и рендер шаблона-родителя.

Если хоть одна проверка не прошла, итог — код 1, и ничего не записано. Код выхода и список `blocked` — как у стороны проекта.

#### Scenario: Обе стороны
- **WHEN** в чистом репозитории лежат `copier.yml` из подмножества и `.copier-answers.yml` с тегом родительского шаблона
- **THEN** появляются `rootstock.toml` и `.rootstock.toml`, `.copier-answers.yml` удалён, `copier.yml` на месте, код 0

#### Scenario: Сторона шаблона не проходит
- **WHEN** в `copier.yml` есть неподдерживаемый ключ, а `.copier-answers.yml` корректен
- **THEN** код 1, ни `rootstock.toml`, ни `.rootstock.toml` не записаны, `.copier-answers.yml` на месте

#### Scenario: Сторона проекта не проходит
- **WHEN** `copier.yml` корректен, а `_commit` в `.copier-answers.yml` не является тегом
- **THEN** код 1, `rootstock.toml` не записан
