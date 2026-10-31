#!/usr/bin/env bash
# Обновление проекта до версии харнесса через rootstock. Зовётся из
# `.github/workflows/harness-update.yml`; логика вынесена сюда, чтобы её можно
# было проверить без GitHub (`.harness/update/tests/update.sh`).
#
# Использование:
#   update.sh <тег>
#       Обновляет проект в текущем каталоге до тега (`rootstock update`)
#       и коммитит одним коммитом «Обновление харнесса до <тег>». Конфликты
#       печатает по одному в строке, `<вид> <путь>`, как rootstock; код 0
#       и с конфликтами. Подпись коммита — из окружения (GIT_AUTHOR_*,
#       GIT_COMMITTER_*).
#
# Команда rootstock — переменная окружения `ROOTSTOCK`, строкой (по умолчанию
# `rootstock`). Workflow ставит закреплённую версию.
set -euo pipefail

[[ $# -eq 1 ]] || {
  printf 'update.sh: использование: update.sh <тег>\n' >&2
  exit 2
}
next=$1

cd "$(git rev-parse --show-toplevel)"

found=$(mktemp)
trap 'rm -f "$found"' EXIT
code=0
# shellcheck disable=SC2086 # команда — строкой, со своими аргументами
${ROOTSTOCK:-rootstock} update --to "$next" --defaults --trust >"$found" || code=$?
# Код 2 — обновление выполнено с конфликтами: это не ошибка, конфликты идут в PR.
case $code in
  0 | 2) ;;
  *)
    printf 'update.sh: rootstock update завершился с кодом %s\n' "$code" >&2
    exit 1
    ;;
esac
git add -A
git commit --quiet --no-verify -m "Обновление харнесса до $next"
sed '/^$/d' "$found" | sort -u
