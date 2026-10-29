#!/usr/bin/env bash
# Хвосты сессии: что осталось незавершённым в этом рабочем дереве.
#
# Правило «доводить change до конца» держалось только скиллом, а хвост копится
# незаметно: за один день сессия оставила два дерева, три ветки на origin
# и основной каталог, отставший от main на два PR. Ничего из этого не видно,
# пока не пойдёшь смотреть руками.
#
# Хук ничего не блокирует и ничего не удаляет: он только называет. Слот
# `session-end` (доклад человеку при выходе); подмешивается и в стартовый
# доклад `session-tree.sh` (доклад модели в начале следующей сессии).
#
# `Stop` для этого не годится: донести текст до модели он может только через
# `decision: "block"`, то есть заставив сессию продолжить ход. Незакоммиченные
# правки — состояние, которое держат намеренно, и такой хук блокировал бы
# каждый выход.
#
# Печатает строки отчёта либо ничего, если хвостов нет. Код возврата всегда 0.
set -uo pipefail

git rev-parse --is-inside-work-tree >/dev/null 2>&1 || exit 0

root=$(git rev-parse --show-toplevel 2>/dev/null) || exit 0
branch=$(git rev-parse --abbrev-ref HEAD 2>/dev/null)
[ -n "$branch" ] || exit 0

lines=()

# --- Незакоммиченные правки ----------------------------------------------------
# `node_modules` симлинкуется в каждое дерево настройкой `worktree` и работой
# сессии не является — иначе любое дерево выглядит грязным с первой секунды.
dirty_paths() {
  git -C "${1:-.}" status --porcelain 2>/dev/null | grep -v '^?? node_modules/\?$'
}

dirty=$(dirty_paths . | grep -c . || true)
if [ "${dirty:-0}" -gt 0 ]; then
  lines+=("незакоммиченных путей: $dirty (git status)")
fi

# --- Незапушенные коммиты ------------------------------------------------------
# Ветка без upstream — тоже хвост: её ещё никто не видел.
if upstream=$(git rev-parse --abbrev-ref --symbolic-full-name '@{upstream}' 2>/dev/null); then
  ahead=$(git rev-list --count "$upstream..HEAD" 2>/dev/null || echo 0)
  if [ "${ahead:-0}" -gt 0 ]; then
    lines+=("коммитов не отправлено: $ahead (ветка $branch, upstream $upstream)")
  fi
else
  base=$(git rev-parse --verify --quiet origin/main >/dev/null 2>&1 && echo origin/main || echo main)
  own=$(git rev-list --count "$base..HEAD" 2>/dev/null || echo 0)
  if [ "${own:-0}" -gt 0 ]; then
    lines+=("ветка $branch не отправлена ни разу, своих коммитов: $own")
  fi
fi

# --- Деревья, чья работа уже в main -------------------------------------------
# Считаем влитым дерево, чьи коммиты не отличаются от main по содержимому:
# PR здесь сливаются squash'ем, поэтому предок — не признак.
if git rev-parse --verify --quiet origin/main >/dev/null 2>&1; then
  main_ref=origin/main
else
  main_ref=main
fi

merged=$(git worktree list --porcelain 2>/dev/null | awk '
  /^worktree /{ wt = substr($0, 10) }
  /^branch /{ br = substr($0, 8); sub(/^refs\/heads\//, "", br); print wt "\t" br }
' | while IFS=$'\t' read -r wt br; do
  [ "$br" = "main" ] && continue
  # Дерево с незакоммиченными правками не трогаем и в отчёт не выносим:
  # оно занято, а не забыто.
  [ -n "$(dirty_paths "$wt")" ] && continue
  ahead=$(git rev-list --count "$main_ref..$br" 2>/dev/null || echo 0)
  [ "${ahead:-0}" -eq 0 ] && continue
  # Два признака «работа уже там»: ветка — предок main (обычное слияние) либо
  # её содержимое совпадает с main (слияние squash'ем, как в этом репозитории).
  # Второй признак перестаёт работать, как только main уходит вперёд, поэтому
  # хук не обещает найти все влитые деревья — только те, что видны наверняка.
  if git merge-base --is-ancestor "$br" "$main_ref" 2>/dev/null \
    || [ -z "$(git diff --stat "$br" "$main_ref" 2>/dev/null)" ]; then
    printf '  %s (%s)\n' "$wt" "$br"
  fi
done)

if [ -n "$merged" ]; then
  lines+=("деревья, чья работа уже в $main_ref, — можно снести:")
  lines+=("$merged")
fi

# --- Расхождения ---------------------------------------------------------------
# `merge-tree --write-tree` проигрывает слияние вхолостую: рабочее дерево
# не трогается, индекс не трогается, ответ — за долю секунды. Сеть тоже
# не трогается, поэтому сравнение идёт с той `origin/main`, что лежит локально.
#
# Заведено 10.09.2026: два PR разошлись по append-only журналам, и узналось
# это в момент, когда оба уже были открыты. Расхождение существовало задолго
# до слияния — его просто никто не спрашивал.
conflicting_files() {
  git merge-tree --write-tree "$1" "$2" 2>/dev/null \
    | sed -n 's/^CONFLICT ([^)]*): Merge conflict in //p'
}

if [ "$branch" != "main" ] && git rev-parse --verify --quiet "$main_ref" >/dev/null 2>&1; then
  with_main=$(conflicting_files "$main_ref" HEAD)
  if [ -n "$with_main" ]; then
    lines+=("расходится с $main_ref по файлам:")
    lines+=("$(printf '  %s\n' $with_main)")
  fi
fi

# Соседние деревья: расхождение видно до того, как чужая ветка попадёт в main.
neighbours=$(git worktree list --porcelain 2>/dev/null | awk -v self="$branch" '
  /^branch /{ br = substr($0, 8); sub(/^refs\/heads\//, "", br)
              if (br != self && br != "main") print br }
')

for br in ${neighbours:-}; do
  files=$(conflicting_files HEAD "$br")
  [ -z "$files" ] && continue
  lines+=("разойдётся с веткой $br по файлам:")
  lines+=("$(printf '  %s\n' $files)")
done

[ ${#lines[@]} -eq 0 ] && exit 0

printf 'Хвосты сессии в %s:\n' "$root"
for l in "${lines[@]}"; do
  case "$l" in
    "  "*) printf '%s\n' "$l" ;;
    *) printf -- '- %s\n' "$l" ;;
  esac
done
printf 'Довести до конца — скилл change-finish.\n'
exit 0
