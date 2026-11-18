#!/usr/bin/env bash
# Hook dispatcher: one entry per slot, common hooks and project hooks.
#
# Usage: dispatch.sh <slot> < event.json
#
# Claude Code, Pi, and Codex call the dispatcher, not hooks one by one:
# this way a project adds its hooks as a file in `.harness/local/hooks/<slot>/`
# without touching the managed wiring. The slot contract is in `README.md` next door.
#
# Order: the slot's common hooks in fixed order (a missing module hook is
# skipped), then `.harness/local/hooks/<slot>/*.sh` alphabetically.
#
# Hook output is merged into one response: the agent can't parse two JSONs in a
# row, and a denial from one of them would get lost silently.
#   - Exit 2 stops the slot: the hook's stderr goes up, exit code 2.
#   - A denial (`permissionDecision: "deny"`, `decision: "block"`,
#     `continue: false`) stops the slot: exactly that JSON goes out.
#   - `additionalContext` of all hooks is joined with a blank line into one
#     JSON; plain hook text goes into the same context.
#   - If nobody produced JSON, plain text is printed as is.
#   - Any other non-zero code is not a denial: stderr goes up, the other hooks
#     run. The final code is 1 if there is no merged JSON (the agent will show the
#     error to the human), and 0 if there is (otherwise the agent would drop the JSON).
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
  subagent-stop) event=SubagentStop common="agent-checks" ;;
  pre-handback) event=PreToolUse common="agent-checks" ;;
  *)
    printf 'dispatch.sh: unknown slot "%s". Slots: session-start, session-end, prompt, pre-bash, post-bash, post-edit, post-edit-async, subagent-stop, pre-handback.\n' "$slot" >&2
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

# The slot's hook list: one path per line.
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
  # A project disables its hook by removing the execute bit — as with git hooks.
  if [ ! -x "$hook" ]; then
    printf 'dispatch.sh: %s is not executable, skipped (chmod +x to enable)\n' "$hook" >&2
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
