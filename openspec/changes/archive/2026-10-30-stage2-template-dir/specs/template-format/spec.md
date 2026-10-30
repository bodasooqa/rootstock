## MODIFIED Requirements

### Requirement: Настройки окружения Jinja и подключения

Шаблон SHALL настраивать `trim_blocks`, `lstrip_blocks`, `keep_trailing_newline`; `keep_trailing_newline` по умолчанию истинен (ТЗ: §Template format, «Jinja environment options»). `{% include %}` и `{% import %}` MUST разрешаться относительно каталога шаблона — каталога с `rootstock.toml`, а не корня шаблона; база подключений не настраивается (ТЗ: §Template format, «Includes», 1.1). Настроенные каталоги частичных шаблонов MUST не попадать в результат. Фильтры и тесты — встроенные minijinja плюс `regex_search` (ТЗ: §Template format).

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
