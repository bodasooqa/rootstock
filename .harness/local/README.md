# Расширения обвязки проекта

`.harness/` целиком принадлежит шаблону и обновляется `rootstock update`, кроме этого каталога: `.harness/local/` создаётся один раз и дальше принадлежит проекту. Обновление шаблона его не трогает.

Правило: проект не правит управляемый файл, а кладёт своё в точку расширения. Тогда обновление почти не конфликтует.

## Точки расширения

| Что | Своё — сюда | Как подключается |
|---|---|---|
| Хуки агента | `.harness/local/hooks/<слот>/<имя>.sh` | диспетчер `.harness/hooks/dispatch.sh` запускает их после общих хуков слота, по алфавиту; зовут его и Claude Code, и прослойка Pi. Слоты и контракт — `.harness/hooks/README.md` |
| Дополнения агентов | `.harness/local/agents/<имя>.md` | агент читает файл, если он есть, — и в `.claude/agents/`, и в `.pi/agents/` |
| Расширения Pi | `.pi/extensions/<имя>.ts` | Pi подключает все расширения каталога; `harness.ts` в нём управляемый |
| Git-хуки | `lefthook.yml` в корне | подключает общий `.harness/lefthook.yml` через `extends`; свои джобы — в нём же |
| CI | `.github/workflows/ci.yml` | зовёт общий `.github/workflows/harness.yml`; свои джобы — рядом |
| biome | `biome.json` в корне | `extends` на `.harness/biome.json` |
| Разрешения Claude Code | `.claude/settings.json` | файл управляемый, но свои строки в `permissions.allow` и `permissions.deny` проект дописывает прямо в него: `rootstock update` сливает правки трёхсторонним слиянием и их сохраняет |
| Настройки Pi | `.pi/settings.json` | так же: файл управляемый, свои ключи проект дописывает прямо в него, слияние их сохраняет |

## Обновления харнесса

Новые версии шаблона приходят PR'ом сами. Три управляемых файла:

- `.github/workflows/harness-update.yml` — раз в сутки (и вручную, Actions → harness-update → Run workflow, с необязательной версией) сверяет теги шаблона с `version` в `.rootstock.toml`. Есть новее — `rootstock update` до неё ([rootstock](https://github.com/bodasooqa/rootstock)), конфликты маркерами в файлах, PR «Обновление харнесса до vX.Y.Z» из ветки `harness/update-vX.Y.Z`. Мажорная версия получает метку `harness:major`, конфликт — `harness:conflict`. Логика — `.harness/update/update.sh`.
- `.github/workflows/harness-automerge.yml` — по завершении CI (workflow `CI` из `ci.yml`; переименуешь его — автовливание перестанет срабатывать) вливает PR обновления squash'ем, если CI зелёный, меток `harness:major` и `harness:conflict` нет, голова PR — проверенный коммит, голову запушил владелец токена (по журналу пушей GitHub, а не по подписи коммита — его не подделать), а все коммиты PR сделаны самим обновлением. PR из форка и PR другого автора не трогает.
- Джоба `markers` в `.github/workflows/harness.yml` — CI красный, пока в файлах есть маркеры конфликта.

Конфликт разбирается как обычная задача: разреши маркеры в ветке PR, закоммить и после зелёного CI влей PR руками — коммит, сделанный не обновлением, автовливание намеренно не пропускает. Мажорную версию вливают руками после шагов миграции из `CHANGELOG.md` шаблона.

Подключение (Settings → Secrets and variables → Actions):

| Что | Вид | Зачем |
|---|---|---|
| `HARNESS_UPDATE_TOKEN` | секрет, обязателен | fine-grained PAT владельца на этот репозиторий: Contents — read and write, Pull requests — read and write, Issues — read and write (метки `harness:major`, `harness:conflict`), Workflows — read and write, Metadata — read. Штатный `GITHUB_TOKEN` не годится: его PR не запускает CI, а пушить правки в `.github/workflows/` ему нельзя. Без секрета оба workflow падают с подсказкой. Автор PR обновления — владелец токена; PR другого автора автовливание не трогает |
| `HARNESS_RUNNER` | переменная, по желанию | раннер обоих workflow: метка строкой (`self-hosted`) или массив меток в JSON (`["self-hosted", "ci"]`); по умолчанию `ubuntu-latest`. На своём раннере нужны `git`, `jq` и `curl`; `gh`, если его нет, workflow сам ставит закреплённой версией в каталог джоба — на Linux x86_64 и arm64. rootstock ставится всегда сам, закреплённой версией со сверкой суммы — на Linux и macOS, x86_64 и arm64 |
| `HARNESS_TEMPLATE_URL` | переменная, по желанию | откуда брать теги шаблона; по умолчанию `https://github.com/bodasooqa/harness`. Обязан совпадать с `source` в `.rootstock.toml` (форма `gh:владелец/репозиторий` тоже годится), иначе обновление остановится с ошибкой |

## Хук проекта

Пример: запрет команды, которая сносит данные проекта.

```bash
#!/usr/bin/env bash
# .harness/local/hooks/pre-bash/guard-data.sh
set -uo pipefail
. "${BASH_SOURCE[0]%/*}/../../../hooks/lib/bash-scan.sh"

scan=$(bash_scan) || exit 0
if printf '%s' "$scan" | grep -Eq 'docker[[:space:]]+volume[[:space:]]+rm'; then
  deny "Том хранит данные, которые не восстанавливаются. Попроси пользователя удалить его самому."
fi
exit 0
```

Файл должен быть исполняемым (`chmod +x`): хук без права на исполнение диспетчер пропускает — так его можно выключить, не удаляя. Тесты своих хуков — тоже здесь, например `.harness/local/hooks/tests/`.
