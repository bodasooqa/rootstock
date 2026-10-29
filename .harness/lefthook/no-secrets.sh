#!/usr/bin/env bash
# pre-commit: не пускает в индекс секреты и артефакты, которых там быть не должно.
set -uo pipefail

fail=0
note() { printf '  %s\n' "$1" >&2; fail=1; }

for f in "$@"; do
  case "$f" in
    .env|.env.*|*/.env|*/.env.*|*.local.toml)
      note "$f — локальный конфиг с секретами, ему место в .gitignore" ;;
    *.db|*.sqlite|*.sqlite3|*.db-shm|*.db-wal)
      note "$f — база данных, в git не коммитится" ;;
  esac
done

# Токены в содержимом: токен Telegram-бота и типичные ключи API.
if [ "$#" -gt 0 ]; then
  hits=$(git diff --cached -U0 -- "$@" \
    | grep -E '^\+' \
    | grep -Ein '[0-9]{8,10}:AA[0-9A-Za-z_-]{33}|(api[_-]?secret|bot[_-]?token|private[_-]?key)[[:space:]]*[=:][[:space:]]*["'"'"'][^"'"'"']{16,}' \
    || true)
  if [ -n "$hits" ]; then
    note "похоже на секрет в добавленных строках:"
    printf '%s\n' "$hits" | head -5 >&2
  fi
fi

if [ "$fail" -ne 0 ]; then
  printf '\nКоммит остановлен. Убери перечисленное из индекса.\n' >&2
  exit 1
fi
exit 0
