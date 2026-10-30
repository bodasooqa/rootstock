## MODIFIED Requirements

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

## ADDED Requirements

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
