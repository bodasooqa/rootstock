#!/usr/bin/env bash
# Слот prompt (UserPromptSubmit): перед созданием change подкладывает открытые
# вопросы, чей срок наступил.
#
# Шаг ритуала «не забудь заглянуть в открытые вопросы» обесценивается первым же
# разом, когда его пролистали, и `.claude/commands/opsx/` всё равно generated —
# дописать шаг туда нельзя. Поэтому не шаг, а справка: вопросы приезжают в контекст
# сами, и пролистать их можно только вместе с заданием.
set -uo pipefail

command -v jq >/dev/null 2>&1 || exit 0

prompt=$(jq -r '.prompt // empty' 2>/dev/null) || exit 0
[ -n "$prompt" ] || exit 0

# Только создание change. `/opsx:apply` и `/opsx:archive` работают с уже
# принятым решением, и справка там — шум.
case "$prompt" in
  *opsx:propose*) ;;
  *) exit 0 ;;
esac

root=$(cd "${BASH_SOURCE[0]%/*}/../.." 2>/dev/null && pwd) || exit 0
due="$root/knowledge/due.sh"
[ -x "$due" ] || exit 0

report=$("$due" 2>/dev/null) || true
[ -n "$report" ] || exit 0

note="Открытые вопросы к этому этапу — их сроки наступили раньше, чем начался новый change:

$report

Вопрос, которого касается предлагаемый change, закрывается или переносится вместе с ним, а не после. Порядок — скилл stage-grooming."

jq -nc --arg n "$note" '{
  hookSpecificOutput: { hookEventName: "UserPromptSubmit", additionalContext: $n }
}'
exit 0
