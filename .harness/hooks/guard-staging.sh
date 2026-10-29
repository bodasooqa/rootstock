#!/usr/bin/env bash
# Слот pre-bash (PreToolUse/Bash): запрещает валовую постановку в индекс
# в общем каталоге.
#
# Две сессии делили один индекс, и `git add` одной унёс в свой коммит файлы
# другой. В своём рабочем дереве индекс собственный, уносить нечего — там
# не мешаем.
set -uo pipefail

. "${BASH_SOURCE[0]%/*}/lib/bash-scan.sh"

# Связанное дерево: git-каталог сессии отличается от общего.
gitdir=$(git rev-parse --git-dir 2>/dev/null) || exit 0
common=$(git rev-parse --git-common-dir 2>/dev/null) || exit 0
[ "$gitdir" = "$common" ] || exit 0

scan=$(bash_scan) || exit 0

# Содержимое кавычек — данные. Иначе `git commit -m "add -a"` читается как флаг.
scan=$(printf '%s' "$scan" | sed -E 's/"[^"]*"/""/g; s/'\''[^'\'']*'\''/'\'\''/g')

wholesale="В общем каталоге индекс делится со всеми сессиями: валовая постановка унесёт в коммит чужие файлы. Ставь названные пути: git add <файл> [<файл>…]. Нужен весь свой набор целиком — заведи рабочее дерево сессии (orca worktree create), там индекс собственный."

if printf '%s' "$scan" | grep -Eq '(^|[;&|[:space:]])git[[:space:]]+add([[:space:]]+-[^;&|[:space:]]+)*[[:space:]]+(\.|\*|:/)([[:space:]]|$)'; then
  deny "$wholesale"
fi

if printf '%s' "$scan" | grep -Eq '(^|[;&|[:space:]])git[[:space:]]+add[^;&|]*[[:space:]](-A|--all|-u|--update)([[:space:]]|$)'; then
  deny "$wholesale"
fi

if printf '%s' "$scan" | grep -Eq '(^|[;&|[:space:]])git[[:space:]]+commit[^;&|]*[[:space:]](-[a-zA-Z]*a[a-zA-Z]*|--all)([[:space:]]|$)'; then
  deny "git commit -a коммитит всё изменённое в общем каталоге, включая правки соседних сессий. Поставь в индекс названные файлы и закоммить без -a."
fi

exit 0
