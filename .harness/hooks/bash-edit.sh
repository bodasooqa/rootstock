#!/usr/bin/env bash
# Слот post-bash (PostToolUse/Bash): ловит правку файла, сделанную командой
# оболочки.
#
# Вся обвязка правок висит на Write|Edit (слоты post-edit и post-edit-async):
# форматирование, проверки по затронутому пакету, проектные хуки. Правка через
# `sed -i` или перенаправление проходит мимо всего этого молча — то есть
# в такой сессии обвязки просто нет. Хук возвращает недостающее напоминание
# и зовёт обратно к Write/Edit; форматирование он не делает сам, чтобы
# не трогать файлы соседних сессий в общем каталоге (на коммите их приводит
# в порядок lefthook).
set -uo pipefail

. "${BASH_SOURCE[0]%/*}/lib/bash-scan.sh"

scan=$(bash_scan) || exit 0

# Каталоги, куда пишут по делу: временные файлы, сборка, зависимости.
scan=$(printf '%s\n' "$scan" | grep -Ev '(/tmp/|scratchpad|/target/|node_modules/|/dev/null)') || exit 0
[ -n "$scan" ] || exit 0

ext='\.(rs|ts|tsx|js|jsx|json|jsonc|toml|md|sql|ya?ml|sh|css)'

hit=""
# Правка на месте.
if printf '%s' "$scan" | grep -Eq "(^|[;&|[:space:]])(sed|perl|ruby|python3?)[[:space:]][^;&|]*-[a-zA-Z]*i[a-zA-Z]*([[:space:]]|')[^;&|]*$ext"; then
  hit="да"
fi
# Перенаправление и tee в файл исходника.
if printf '%s' "$scan" | grep -Eq "(>>?[[:space:]]*[^;&|[:space:]]*$ext|(^|[;&|[:space:]])tee[[:space:]][^;&|]*$ext)"; then
  hit="да"
fi

[ -n "$hit" ] || exit 0

hooks_text="rustfmt, clippy по затронутому крейту, проектные хуки .harness/local/hooks/post-edit"
manual="rustfmt --edition 2024 <файл>, затем cargo clippy --all-targets -- -D warnings"

jq -nc --arg n "Похоже, файл правился командой оболочки. На Write/Edit висит вся обвязка правок: $hooks_text. Через Bash не срабатывает ничего из этого. Правь файлы инструментами Write и Edit; Bash оставляй для команд. Если правка уже сделана — прогони вручную то, что пропущено: $manual." '{
  hookSpecificOutput: { hookEventName: "PostToolUse", additionalContext: $n }
}'
exit 0
