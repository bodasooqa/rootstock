#!/usr/bin/env bash
# pre-commit: keeps secrets and artifacts that don't belong there out of the index.
set -uo pipefail

fail=0
note() { printf '  %s\n' "$1" >&2; fail=1; }

for f in "$@"; do
  case "$f" in
    .env|.env.*|*/.env|*/.env.*|*.local.toml)
      note "$f — local config with secrets, it belongs in .gitignore" ;;
    *.db|*.sqlite|*.sqlite3|*.db-shm|*.db-wal)
      note "$f — database, not committed to git" ;;
  esac
done

# Tokens in content: a Telegram bot token and typical API keys.
if [ "$#" -gt 0 ]; then
  hits=$(git diff --cached -U0 -- "$@" \
    | grep -E '^\+' \
    | grep -Ein '[0-9]{8,10}:AA[0-9A-Za-z_-]{33}|(api[_-]?secret|bot[_-]?token|private[_-]?key)[[:space:]]*[=:][[:space:]]*["'"'"'][^"'"'"']{16,}' \
    || true)
  if [ -n "$hits" ]; then
    note "looks like a secret in added lines:"
    printf '%s\n' "$hits" | head -5 >&2
  fi
fi

if [ "$fail" -ne 0 ]; then
  printf '\nCommit stopped. Remove the listed items from the index.\n' >&2
  exit 1
fi
exit 0
