#!/usr/bin/env bash
# Slot post-bash (PostToolUse/Bash): catches a file edit made by a shell
# command.
#
# The whole edit harness hangs on Write|Edit (slots post-edit and post-edit-async):
# formatting, checks for the affected package, project hooks. An edit via
# `sed -i` or a redirect silently bypasses all of it — meaning that in such a
# session there is no harness at all. The hook returns the missing reminder
# and calls back to Write/Edit; it doesn't format by itself, so as not to touch
# neighbouring sessions' files in the shared checkout (lefthook tidies them
# up on commit).
set -uo pipefail

. "${BASH_SOURCE[0]%/*}/lib/bash-scan.sh"

scan=$(bash_scan) || exit 0

# Directories written to legitimately: temporary files, build output, dependencies.
scan=$(printf '%s\n' "$scan" | grep -Ev '(/tmp/|scratchpad|/target/|node_modules/|/dev/null)') || exit 0
[ -n "$scan" ] || exit 0

ext='\.(rs|ts|tsx|js|jsx|json|jsonc|toml|md|sql|ya?ml|sh|css)'

hit=""
# In-place edit.
if printf '%s' "$scan" | grep -Eq "(^|[;&|[:space:]])(sed|perl|ruby|python3?)[[:space:]][^;&|]*-[a-zA-Z]*i[a-zA-Z]*([[:space:]]|')[^;&|]*$ext"; then
  hit="yes"
fi
# Redirect and tee into a source file.
if printf '%s' "$scan" | grep -Eq "(>>?[[:space:]]*[^;&|[:space:]]*$ext|(^|[;&|[:space:]])tee[[:space:]][^;&|]*$ext)"; then
  hit="yes"
fi

[ -n "$hit" ] || exit 0

hooks_text="rustfmt, clippy for the affected crate, project hooks .harness/local/hooks/post-edit"
manual="rustfmt --edition 2024 <file>, then cargo clippy --all-targets -- -D warnings"

jq -nc --arg n "Looks like a file was edited by a shell command. The whole edit harness hangs on Write/Edit: $hooks_text. None of it fires through Bash. Edit files with the Write and Edit tools; keep Bash for commands. If the edit is already made, run what was skipped by hand: $manual." '{
  hookSpecificOutput: { hookEventName: "PostToolUse", additionalContext: $n }
}'
exit 0
