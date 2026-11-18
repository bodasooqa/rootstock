#!/usr/bin/env bash
# Slot prompt (UserPromptSubmit): before a change is created, injects the open
# questions that have come due.
#
# A ritual step "don't forget to look at the open questions" loses its value the
# first time it is skimmed past, and `.claude/commands/opsx/` is generated anyway —
# a step can't be added there. So not a step but a reference: the questions arrive
# in the context by themselves, and skipping them means skipping the task too.
set -uo pipefail

command -v jq >/dev/null 2>&1 || exit 0

prompt=$(jq -r '.prompt // empty' 2>/dev/null) || exit 0
[ -n "$prompt" ] || exit 0

# Only change creation. `/opsx:apply` and `/opsx:archive` work on a decision
# already made, and the reference there is noise.
case "$prompt" in
  *opsx:propose*|*opsx-propose*|*'$openspec-propose'*) ;;
  *) exit 0 ;;
esac

root=$(cd "${BASH_SOURCE[0]%/*}/../.." 2>/dev/null && pwd) || exit 0
due="$root/knowledge/due.sh"
[ -x "$due" ] || exit 0

report=$("$due" 2>/dev/null) || true
[ -n "$report" ] || exit 0

note="Open questions for this stage — they came due before the new change started:

$report

A question the proposed change touches is closed or moved together with it, not afterwards. The procedure is the stage-grooming skill."

jq -nc --arg n "$note" '{
  hookSpecificOutput: { hookEventName: "UserPromptSubmit", additionalContext: $n }
}'
exit 0
