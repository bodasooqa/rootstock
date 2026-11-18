#!/usr/bin/env bash
set -euo pipefail

hooks=$(cd "${BASH_SOURCE[0]%/*}/.." && pwd)
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
mkdir -p "$tmp/.harness/hooks" "$tmp/project with spaces"
cp "$hooks/codex.sh" "$tmp/.harness/hooks/"
cd "$tmp/project with spaces"

# Record the events reaching the dispatcher without running project checks.
cat >"$tmp/.harness/hooks/dispatch.sh" <<'SH'
#!/usr/bin/env bash
set -euo pipefail
input=$(cat)
jq -cn --arg slot "$1" --argjson event "$input" '{slot:$slot,event:$event}' >>"$EVENTS"
case "$1" in
  pre-bash)
    if [[ $(jq -r '.tool_input.command' <<<"$input") == 'git add -A' ]]; then
      echo '{"hookSpecificOutput":{"hookEventName":"PreToolUse","permissionDecision":"deny","permissionDecisionReason":"stage by name"}}'
    fi
    ;;
  post-edit)
    jq -c '{hookSpecificOutput:{hookEventName:"PostToolUse",additionalContext:.tool_input.file_path}}' <<<"$input"
    ;;
  post-edit-async)
    if [[ $(jq -r '.tool_input.file_path' <<<"$input") == a.rs ]]; then
      echo "clippy failed after edit" >&2
      exit 2
    fi
    ;;
  subagent-stop)
    if [[ $(jq -r '.stop_hook_active' <<<"$input") == false ]]; then
      echo '{"decision":"block","reason":"clippy failed"}'
    fi
    ;;
esac
SH
chmod +x "$tmp/.harness/hooks/dispatch.sh"
export EVENTS="$tmp/events.jsonl"
adapter="$tmp/.harness/hooks/codex.sh"

allowed='{"hook_event_name":"PreToolUse","tool_name":"Bash","tool_input":{"command":"git status"}}'
test -z "$(bash "$adapter" pre-bash <<<"$allowed")"
blocked='{"hook_event_name":"PreToolUse","tool_name":"Bash","tool_input":{"command":"git add -A"}}'
bash "$adapter" pre-bash <<<"$blocked" | jq -e '.hookSpecificOutput.permissionDecision == "deny"' >/dev/null

patch='*** Begin Patch
*** Update File: a.rs
@@
-before
+after
*** Update File: old.ts
*** Move to: directory with spaces/new.ts
@@
-old
+*** Delete File: not-a-header
*** Delete File: removed.rs
*** Add File: new.ts
+export {}
*** End Patch'
git init -q
jq -cn --arg patch "$patch" --arg cwd "$PWD" '{cwd:$cwd,hook_event_name:"PostToolUse",tool_name:"apply_patch",tool_input:{command:$patch}}' |
  bash "$adapter" post-edit |
  jq -e '.hookSpecificOutput.additionalContext ==
    "a.rs\n\nold.ts\n\ndirectory with spaces/new.ts\n\nremoved.rs\n\nnew.ts\n\nclippy failed after edit"' >/dev/null
jq -es '[.[] | select(.slot | startswith("post-edit")) | .slot] ==
  ([range(5) | "post-edit"] + [range(5) | "post-edit-async"])' "$EVENTS" >/dev/null

developer='{"hook_event_name":"SubagentStop","agent_id":"a1","agent_type":"rust-developer","stop_hook_active":false}'
bash "$adapter" subagent-stop <<<"$developer" | jq -e '.decision == "block" and .reason == "clippy failed"' >/dev/null
test -z "$(jq '.stop_hook_active = true' <<<"$developer" | bash "$adapter" subagent-stop)"
test -z "$(jq '.agent_type = "rust-reviewer"' <<<"$developer" | bash "$adapter" subagent-stop)"
jq -es '[.[] | select(.slot == "subagent-stop")] | length == 2' "$EVENTS" >/dev/null

echo "Codex adapter checks passed"
