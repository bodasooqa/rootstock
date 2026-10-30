## MODIFIED Requirements

### Requirement: Настройки окружения Jinja и подключения

Шаблон SHALL настраивать `trim_blocks`, `lstrip_blocks`, `keep_trailing_newline`; `keep_trailing_newline` по умолчанию истинен (ТЗ: §Template format, «Jinja environment options»). `{% include %}` и `{% import %}` MUST разрешаться относительно каталога шаблона — каталога с `rootstock.toml`, а не корня шаблона; база подключений не настраивается (ТЗ: §Template format, «Includes», 1.1). Настроенные каталоги частичных шаблонов MUST не попадать в результат. Фильтры и тесты — встроенные minijinja плюс `regex_search` (ТЗ: §Template format).

На значениях SHALL вызываться Python-методы с семантикой Python (ТЗ: §Template format, 1.2):
- строк — `startswith`, `endswith`, `lower`, `upper`, `strip`, `lstrip`, `rstrip`, `split`, `replace` и другие из набора design D1;
- словарей — `items`, `keys`, `values`, `get`;
- списков — `count` и `append`.

`append` у списка, заведённого `{% set x = [] %}`, MUST менять этот список на месте: добавленное видно после цикла и после выхода из блока, где вызван метод (ТЗ: §Template format, 1.2). `append` у другого списка и метод вне набора MUST быть ошибкой рендера (design D3). Метод MUST вызываться одинаково в содержимом файлов, в именах, в подключаемых файлах, в `default`, `validator` и в правилах классов файлов (design D4).

#### Scenario: Завершающий перевод строки сохраняется
- **WHEN** шаблонизируемый файл заканчивается переводом строки, а настройки не менялись
- **THEN** отрендеренный файл заканчивается тем же переводом строки

#### Scenario: Частичный шаблон не выводится
- **WHEN** каталог `partials` указан как каталог частичных шаблонов и из файла сделан `{% include "partials/a.jinja" %}`
- **THEN** содержимое включается в результат, а самого каталога `partials` в проекте нет

#### Scenario: Подключение от каталога шаблона
- **WHEN** корень шаблона — `template/`, а файл в нём делает `{% include "template/_partials/x.jinja" %}`
- **THEN** подключается `template/_partials/x.jinja` из каталога с `rootstock.toml`

#### Scenario: Ключ include_root отклоняется
- **WHEN** `rootstock.toml` содержит ключ `include_root`
- **THEN** конфигурация отклоняется как содержащая неизвестный ключ, код 1 (design D5)

#### Scenario: Строковый метод
- **WHEN** файл содержит `{{ path[:-3] if path.endswith('.md') else path }}`, а ответ `path` — `k/oq.md`
- **THEN** в проекте `k/oq`

#### Scenario: Список, собранный append во вложенных циклах
- **WHEN** макрос заводит `{% set names = [] %}`, во вложенном цикле вызывает `{% set _ = names.append(name) %}`, затем выводит `{{ names | join(", ") }}`
- **THEN** выводятся все добавленные имена в порядке добавления

#### Scenario: append у другого списка
- **WHEN** файл вызывает `append` у списка `{% set x = [1] %}`
- **THEN** рендер завершается ошибкой, код 1, ничего не записано

#### Scenario: Шаблон с set в raw-блоке
- **WHEN** в `{% raw %}…{% endraw %}` стоит текст `{% set x = [] %}`
- **THEN** этот текст выводится как есть
