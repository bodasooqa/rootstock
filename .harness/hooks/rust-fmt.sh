#!/usr/bin/env bash
# Slot post-edit (PostToolUse/Write|Edit), module rust: formats the .rs file that
# was just written.
set -uo pipefail

f=$(jq -r '.tool_response.filePath // .tool_input.file_path // empty' 2>/dev/null) || exit 0
case "$f" in
  *.rs) ;;
  *) exit 0 ;;
esac
[ -f "$f" ] || exit 0

rustfmt --edition 2024 "$f" 2>/dev/null || true
exit 0
