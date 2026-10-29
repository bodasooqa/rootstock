#!/usr/bin/env bash
# Слот session-start (SessionStart): докладывает, где сессия работает и кто ещё
# занят в репозитории.
#
# Правило «своё дерево на сессию» держалось только прозой CLAUDE.md, а цена его
# нарушения — чужие файлы в своём коммите. Сессия должна узнать обстановку
# до первой правки, а не после чужого коммита. Хук смотрит только `git worktree`
# и никаких менеджеров деревьев не вызывает.
set -uo pipefail

git rev-parse --is-inside-work-tree >/dev/null 2>&1 || exit 0

root=$(git rev-parse --show-toplevel 2>/dev/null) || exit 0
gitdir=$(git rev-parse --git-dir 2>/dev/null)
common=$(git rev-parse --git-common-dir 2>/dev/null)
branch=$(git rev-parse --abbrev-ref HEAD 2>/dev/null)

if [ "$gitdir" = "$common" ]; then
  where="Сессия работает в ОБЩЕМ каталоге $root (ветка $branch). Индекс здесь делится со всеми сессиями."
  own="нет"
else
  where="Сессия работает в своём дереве $root (ветка $branch)."
  own="да"
fi

# Чужие деревья: всё из git worktree list, кроме текущего.
others=$(git worktree list --porcelain 2>/dev/null | awk -v self="$root" '
  /^worktree /{ wt = substr($0, 10) }
  /^branch /{ br = substr($0, 8); sub(/^refs\/heads\//, "", br)
              if (wt != self) printf "  %s (%s)\n", wt, br }
')

note="$where"
if [ -n "$others" ]; then
  note="$note"$'\n'"Другие рабочие деревья этого репозитория:"$'\n'"$others"
fi

# Общий каталог сборки на все деревья не годится: пакет во всех деревьях
# называется одинаково, а код в них разный, и сборка одного дерева ложится
# на место другого. Конфиг машинно-зависим и не коммитится, поэтому сам он
# не исчезнет — на машине, где он остался, зелёный прогон в дереве ничего
# не доказывает.
common_abs=$(cd "$common" 2>/dev/null && pwd)
if [ -n "$common_abs" ]; then
  cargo_config="$(dirname "$common_abs")/.cargo/config.toml"
  if grep -qs '^[[:space:]]*target-dir' "$cargo_config"; then
    note="$note"$'\n'"Общий каталог сборки настроен: в $cargo_config стоит target-dir. Деревья с общим каталогом затирают сборки друг друга. Убрать: rm -f $cargo_config."
  fi
fi

# Git-хуки общие на все деревья, а lefthook вписывает в них абсолютный путь к
# своему бинарю. Установка из дерева (postinstall пакета lefthook при `bun
# install`) уводит путь в node_modules этого дерева; дерево сносят — и
# pre-commit с pre-push молча перестают работать во всём репозитории. Путь
# в node_modules обязан вести в основной каталог; путь вне node_modules
# (lefthook стоит глобально) обязан существовать.
main_dir=$(cd "$common/.." 2>/dev/null && pwd -P)
if [ -n "$main_dir" ] && [ -f "$main_dir/lefthook.yml" ]; then
  hooks_dir=$(git rev-parse --path-format=absolute --git-path hooks 2>/dev/null)
  broken=""
  for h in pre-commit pre-push; do
    hook="$hooks_dir/$h"
    bin=$(sed -n 's|^[[:space:]]*elif \(/[^ ]*/lefthook\) -h.*|\1|p' "$hook" 2>/dev/null | head -n 1)
    case "$bin" in
      "") broken="$broken"$'\n'"  $hook — нет пути к lefthook" ;;
      "$main_dir/node_modules/"*) [ -x "$bin" ] || broken="$broken"$'\n'"  $hook → $bin — файла нет" ;;
      */node_modules/*) broken="$broken"$'\n'"  $hook → $bin — не основной каталог" ;;
      *) [ -x "$bin" ] || broken="$broken"$'\n'"  $hook → $bin — файла нет" ;;
    esac
  done
  if [ -n "$broken" ]; then
    note="$note"$'\n'"Git-хуки сломаны, pre-commit и pre-push не работают:$broken"$'\n'"Почини: cd $main_dir && lefthook install"
  fi
fi

# Просроченные открытые вопросы. Одна строка, без перечисления: список печатает
# `knowledge/due.sh`, а доклад лишь не даёт долгу быть незаметным.
due="$root/knowledge/due.sh"
if [ -x "$due" ]; then
  overdue=$("$due" --overdue 2>/dev/null | sed -n 's/^Просрочено вопросов: \([0-9]*\)$/\1/p')
  if [ -n "$overdue" ]; then
    note="$note"$'\n'"Просроченных открытых вопросов: $overdue. Список — knowledge/due.sh --overdue, разбор — скилл stage-grooming."
  fi
fi

# Хвосты прошлой сессии: тот же текст, что уходит человеку на выходе, только
# здесь он приезжает модели на входе — прибирать удобнее в начале, а не в конце.
leftovers="${BASH_SOURCE[0]%/*}/session-leftovers.sh"
if [ -x "$leftovers" ]; then
  tails=$("$leftovers" 2>/dev/null)
  [ -n "$tails" ] && note="$note"$'\n'"$tails"
fi

if [ "$own" = "нет" ]; then
  note="$note"$'\n'"Правка файлов из общего каталога разрешена только когда других сессий по этому репозиторию заведомо нет. Иначе заведи своё дерево: orca worktree create. Валовая постановка в индекс здесь запрещена хуком — ставь названные пути."
fi

jq -nc --arg n "$note" '{
  hookSpecificOutput: { hookEventName: "SessionStart", additionalContext: $n }
}'
exit 0
