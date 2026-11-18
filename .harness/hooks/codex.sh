#!/usr/bin/env bash
# Translate Codex hook events to the shared dispatcher contract.
set -euo pipefail

here=$(cd "${BASH_SOURCE[0]%/*}" && pwd)
slot=${1:?missing hook slot}
input=$(cat)
event=$(jq -r '.hook_event_name' <<<"$input")

run_slot() {
  local output status=0
  if [[ $1 == post-edit-async ]]; then
    # The shared slot uses exit 2 and stderr to wake Claude Code. Codex
    # receives the same feedback as context in the current tool response.
    output=$("$here/dispatch.sh" "$1" 2>&1) || status=$?
    if [[ $status == 2 ]]; then
      jq -cn --arg output "$output" \
        '{hookSpecificOutput:{hookEventName:"PostToolUse",additionalContext:$output}}'
      return
    fi
    if [[ $status != 0 ]]; then
      printf '%s\n' "$output" >&2
      return "$status"
    fi
  else
    output=$("$here/dispatch.sh" "$1") || status=$?
  fi
  if [[ $status != 0 || -z $output ]]; then
    return "$status"
  fi
  if jq -e 'type == "object"' >/dev/null 2>&1 <<<"$output"; then
    printf '%s\n' "$output"
  elif [[ $event == SessionEnd ]]; then
    jq -cn --arg output "$output" '{systemMessage:$output}'
  else
    jq -cn --arg event "$event" --arg output "$output" \
      '{hookSpecificOutput:{hookEventName:$event,additionalContext:$output}}'
  fi
}

case "$slot" in
  post-edit)
    # Patch body lines carry a diff prefix. Only unprefixed directives identify paths.
    paths=$(jq -r '.tool_input.command | strings | split("\n")[] |
      capture("^\\*\\*\\* ((Add|Delete|Update) File|Move to): (?<path>.+)$").path' <<<"$input" |
      awk '!seen[$0]++')
    [[ -n $paths ]] || exit 0

    tmp=$(mktemp)
    trap 'rm -f "$tmp"' EXIT
    for check in post-edit post-edit-async; do
      while IFS= read -r path; do
        jq -c --arg path "$path" '.tool_input.file_path = $path | del(.tool_response)' <<<"$input" |
          run_slot "$check" >>"$tmp"
      done <<<"$paths"
    done
    # Codex accepts one JSON object per invocation, even for a multi-file patch.
    jq -sc '
      (map(select(.decision == "block" or .continue == false)) | first) //
      (map(.hookSpecificOutput.additionalContext // empty) |
      reduce .[] as $item ([]; if index($item) then . else . + [$item] end) |
      if length == 0 then empty else
        {hookSpecificOutput:{hookEventName:"PostToolUse",additionalContext:join("\n\n")}}
      end)
    ' "$tmp"
    ;;
  subagent-stop)
    # Keep project stop hooks scoped to executors, as on the other clients.
    [[ $(jq -r '.agent_type' <<<"$input") == *-developer ]] || exit 0
    run_slot "$slot" <<<"$input"
    ;;
  *)
    run_slot "$slot" <<<"$input"
    ;;
esac
