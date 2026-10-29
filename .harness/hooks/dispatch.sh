#!/usr/bin/env bash
# Диспетчер хуков: один вход на слот, общие хуки и хуки проекта.
#
# Использование: dispatch.sh <слот> < событие.json
#
# Агент (Claude Code, прослойка Pi `.pi/extensions/harness.ts`) зовёт диспетчер, а не хуки по одному:
# так проект добавляет свои хуки файлом в `.harness/local/hooks/<слот>/`,
# не трогая управляемую проводку. Контракт слота — `README.md` рядом.
#
# Порядок: общие хуки слота в фиксированном порядке (отсутствующий модульный
# хук пропускается), затем `.harness/local/hooks/<слот>/*.sh` по алфавиту.
#
# Вывод хуков сводится в один ответ: два JSON подряд агент не разберёт,
# и отказ одного из них потерялся бы молча.
#   - Код 2 останавливает слот: stderr хука уходит наверх, код выхода 2.
#   - Отказ (`permissionDecision: "deny"`, `decision: "block"`,
#     `continue: false`) останавливает слот: наружу уходит ровно этот JSON.
#   - `additionalContext` всех хуков склеивается через пустую строку в один
#     JSON; простой текст хуков идёт в тот же контекст.
#   - Если JSON не было ни у кого, простой текст печатается как есть.
#   - Прочий ненулевой код — не отказ: stderr уходит наверх, остальные хуки
#     работают. Итоговый код 1, если сводного JSON нет (агент покажет ошибку
#     человеку), и 0, если есть (иначе агент выбросил бы JSON).
set -uo pipefail

slot=${1:-}
case "$slot" in
  session-start) event=SessionStart common="session-tree" ;;
  session-end) event=SessionEnd common="session-leftovers" ;;
  prompt) event=UserPromptSubmit common="knowledge-due" ;;
  pre-bash) event=PreToolUse common="guard-staging" ;;
  post-bash) event=PostToolUse common="bash-edit" ;;
  post-edit) event=PostToolUse common="rust-fmt js-fmt" ;;
  post-edit-async) event=PostToolUse common="rust-clippy js-typecheck" ;;
  *)
    printf 'dispatch.sh: неизвестный слот «%s». Слоты: session-start, session-end, prompt, pre-bash, post-bash, post-edit, post-edit-async.\n' "$slot" >&2
    exit 1
    ;;
esac

here=$(cd "${BASH_SOURCE[0]%/*}" && pwd)
local_dir="$here/../local/hooks/$slot"

tmp=$(mktemp -d "${TMPDIR:-/tmp}/harness-dispatch.XXXXXX") || exit 1
trap 'rm -rf "$tmp"' EXIT
input="$tmp/input"
errf="$tmp/stderr"
cat >"$input"

# Список хуков слота: путь на строку.
hooks=""
for name in $common; do
  [ -f "$here/$name.sh" ] && hooks="$hooks$here/$name.sh"$'\n'
done
if [ -d "$local_dir" ]; then
  hooks="$hooks$(find "$local_dir" -maxdepth 1 -name '*.sh' \( -type f -o -type l \) 2>/dev/null | LC_ALL=C sort)"$'\n'
fi

merged='{}'
have_json=0
failed=0
context=""
plain=""

add_context() {
  if [ -n "$context" ]; then
    context="$context"$'\n\n'"$1"
  else
    context=$1
  fi
}

while IFS= read -r hook; do
  [ -n "$hook" ] || continue
  # Проект выключает свой хук снятием права на исполнение — как у git-хуков.
  if [ ! -x "$hook" ]; then
    printf 'dispatch.sh: %s не исполняемый, пропущен (chmod +x, чтобы включить)\n' "$hook" >&2
    continue
  fi

  out=$("$hook" <"$input" 2>"$errf")
  code=$?
  if [ "$code" -eq 2 ]; then
    cat "$errf" >&2
    exit 2
  fi
  [ -s "$errf" ] && cat "$errf" >&2
  [ "$code" -eq 0 ] || failed=1
  [ -n "$out" ] || continue

  if printf '%s' "$out" | jq -e 'type == "object"' >/dev/null 2>&1; then
    if printf '%s' "$out" | jq -e '
      (.hookSpecificOutput.permissionDecision? == "deny")
      or (.decision? == "block")
      or (.continue? == false)
    ' >/dev/null 2>&1; then
      printf '%s\n' "$out"
      exit 0
    fi
    ctx=$(printf '%s' "$out" | jq -r '.hookSpecificOutput.additionalContext // empty')
    [ -n "$ctx" ] && add_context "$ctx"
    merged=$(printf '%s' "$out" | jq -c --argjson acc "$merged" \
      '$acc * (del(.hookSpecificOutput.additionalContext) | if .hookSpecificOutput == {} then del(.hookSpecificOutput) else . end)')
    have_json=1
  else
    add_context "$out"
    if [ -n "$plain" ]; then
      plain="$plain"$'\n'"$out"
    else
      plain=$out
    fi
  fi
done <<EOF
$hooks
EOF

if [ "$have_json" -eq 1 ]; then
  printf '%s' "$merged" | jq -c --arg ctx "$context" --arg event "$event" '
    if $ctx == "" then .
    else .hookSpecificOutput.hookEventName = (.hookSpecificOutput.hookEventName // $event)
       | .hookSpecificOutput.additionalContext = $ctx
    end'
  exit 0
fi

[ -n "$plain" ] && printf '%s\n' "$plain"
exit "$failed"
