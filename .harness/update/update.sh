#!/usr/bin/env bash
# Обновление проекта до версии харнесса: copier или rootstock — по формату
# целевого тега. Зовётся из `.github/workflows/harness-update.yml`; логика
# вынесена сюда, чтобы её можно было проверить без GitHub
# (`.harness/update/tests/update.sh`).
#
# Использование:
#   update.sh format <тег>
#       Печатает `rootstock`, если тег шаблона несёт `rootstock.toml` в корне,
#       иначе `copier`. Шаблон — $TEMPLATE_URL. Код 2 — тега нет.
#   update.sh apply copier|rootstock <тег>
#       Обновляет проект в текущем каталоге до тега инструментом, который
#       назвал `format`, и коммитит. Конфликты печатает по одному в строке,
#       `<вид> <путь>`, как rootstock; код 0 и с конфликтами. Подпись
#       коммитов — из окружения (GIT_AUTHOR_*, GIT_COMMITTER_*).
#       - copier: `copier update`, конфликты маркерами в файлах (вид
#         `markers`), один коммит «Обновление харнесса до <тег>».
#       - rootstock: проект ещё на copier (`.copier-answers.yml`), поэтому
#         сначала `rootstock import-copier` отдельным коммитом «Перевод
#         харнесса на rootstock», затем `rootstock update` вторым.
#
# Команды — переменные окружения, строкой: `COPIER` (по умолчанию `copier`),
# `ROOTSTOCK` (по умолчанию `rootstock`). Workflow подставляет закреплённые
# версии.
set -euo pipefail

die() {
  printf 'update.sh: %s\n' "$1" >&2
  exit 2
}

# Код 0, если у тега $1 шаблона $TEMPLATE_URL в корне есть rootstock.toml.
# Тег скачивается без истории во временный репозиторий: проект не трогается.
ships_rootstock() {
  local tmp code=0
  tmp=$(mktemp -d)
  git -C "$tmp" init -q
  if ! git -C "$tmp" fetch -q --depth 1 "$TEMPLATE_URL" "refs/tags/$1" 2>/dev/null; then
    rm -rf "$tmp"
    die "нет тега $1 в $TEMPLATE_URL"
  fi
  git -C "$tmp" cat-file -e 'FETCH_HEAD^{tree}:rootstock.toml' 2>/dev/null || code=1
  rm -rf "$tmp"
  return "$code"
}

commit() {
  git add -A
  git commit --quiet --no-verify -m "$1"
}

apply_copier() {
  local next=$1 copier=${COPIER:-copier} unmerged markers
  # Конфликт copier оставляет маркерами в файлах (--conflict inline)
  # и помечает файл неслитым в индексе.
  # shellcheck disable=SC2086 # команда — строкой, со своими аргументами
  $copier update --trust --defaults --conflict inline --vcs-ref "$next" >&2
  unmerged=$(git diff --name-only --diff-filter=U)
  git add -A
  markers=$(.harness/update/conflict-markers.sh 2>/dev/null || true)
  commit "Обновление харнесса до $next"
  printf '%s\n%s\n' "$unmerged" "$markers" | sed '/^$/d' | sort -u | sed 's/^/markers /'
}

# Запуск rootstock: конфликты (stdout) — в $1, код 2 — не ошибка.
rootstock_run() {
  local out=$1 code=0
  shift
  # shellcheck disable=SC2086 # команда — строкой, со своими аргументами
  ${ROOTSTOCK:-rootstock} "$@" >"$out" || code=$?
  case $code in
    0 | 2) ;;
    *)
      printf 'update.sh: rootstock %s завершился с кодом %s\n' "$1" "$code" >&2
      exit 1
      ;;
  esac
}

apply_rootstock() {
  local next=$1 found
  found=$(mktemp -d)
  # Импорт пишет `.rootstock.toml` на версии проекта и убирает
  # `.copier-answers.yml`; файлы проекта не меняются. Отдельный коммит: его
  # видно в PR отдельно от самого обновления, а update требует чистое дерево.
  rootstock_run "$found/import" import-copier
  commit "Перевод харнесса на rootstock"
  rootstock_run "$found/update" update --to "$next" --defaults --trust
  commit "Обновление харнесса до $next"
  cat "$found/import" "$found/update" | sed '/^$/d' | sort -u
  rm -rf "$found"
}

usage="использование: update.sh format <тег> | update.sh apply copier|rootstock <тег>"
case "${1:-}/$#" in
  format/2)
    [ -n "${TEMPLATE_URL:-}" ] || die "не задан TEMPLATE_URL"
    if ships_rootstock "$2"; then echo rootstock; else echo copier; fi
    ;;
  apply/3)
    cd "$(git rev-parse --show-toplevel)"
    case $2 in
      copier) apply_copier "$3" ;;
      rootstock) apply_rootstock "$3" ;;
      *) die "$usage" ;;
    esac
    ;;
  *) die "$usage" ;;
esac
