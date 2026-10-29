#!/usr/bin/env bash
# Тесты `knowledge/archive-questions.sh`: закрытый вопрос уезжает в архив,
# открытый остаётся; без закрытых — файлы не трогаются; `--stage` ставит
# в индекс оба файла. Проект синтетический, во временном каталоге.
#
# Запуск: knowledge/tests/archive-questions.sh
set -uo pipefail

# Под git-хуком в окружении стоят GIT_DIR и прочие: git смотрел бы в репозиторий
# хука, а не во временный.
while IFS='=' read -r name _; do
  case "$name" in GIT_*) unset "$name" ;; esac
done < <(env)

here=$(cd "${BASH_SOURCE[0]%/*}" && pwd)
script="$here/../archive-questions.sh"

pass=0
fail=0
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

ok()  { pass=$((pass + 1)); printf '  ok   %s\n' "$1"; }
bad() { fail=$((fail + 1)); printf '  ПРОВАЛ %s\n     ожидалось: %s\n     получено: %s\n' "$1" "$2" "${3:-<пусто>}"; }

# setup <путь вопросов> — проект с файлом ответов и двумя вопросами.
setup() {
  local questions=$1
  rm -rf "$tmp/repo"
  mkdir -p "$tmp/repo/knowledge" "$tmp/repo/$(dirname "$questions")"
  cp "$script" "$tmp/repo/knowledge/"
  printf 'open_questions_path: %s\n' "$questions" > "$tmp/repo/.copier-answers.yml"
  printf '# Открытые вопросы\n\nГолова.\n\n---\n\n## Первый вопрос\n\nСуть.\n\n---\n\n## Второй вопрос\n\nСуть.\n' \
    > "$tmp/repo/$questions"
}

echo "archive-questions.sh"

setup knowledge/open-questions.md
before=$(cat "$tmp/repo/knowledge/open-questions.md")
out=$("$tmp/repo/knowledge/archive-questions.sh" 2>&1); code=$?
{ [ $code -eq 0 ] && [ -z "$out" ] && [ "$before" = "$(cat "$tmp/repo/knowledge/open-questions.md")" ] \
  && [ ! -e "$tmp/repo/knowledge/open-questions-closed.md" ]; } \
  && ok "без закрытых ничего не трогает" || bad "без закрытых ничего не трогает" "молчание, файлы на месте" "код $code: $out"

perl -i -pe 's/^## Первый вопрос$/## ~~Первый вопрос~~ — закрыт 08.10.2026/' "$tmp/repo/knowledge/open-questions.md"
out=$("$tmp/repo/knowledge/archive-questions.sh" 2>&1); code=$?
open=$(cat "$tmp/repo/knowledge/open-questions.md")
closed=$(cat "$tmp/repo/knowledge/open-questions-closed.md" 2>/dev/null)
{ [ $code -eq 0 ] && [[ "$open" != *"Первый вопрос"* ]] && [[ "$open" == *"## Второй вопрос"* ]] \
  && [[ "$closed" == *"## ~~Первый вопрос~~"* ]] && [[ "$closed" != *"Второй вопрос"* ]]; } \
  && ok "закрытый уезжает в архив, открытый остаётся" \
  || bad "закрытый уезжает в архив, открытый остаётся" "Первый — в архиве, Второй — на месте" "код $code: $out / $open / $closed"

# Повторный прогон: переносить нечего, архив не растёт.
closed_before=$closed
"$tmp/repo/knowledge/archive-questions.sh" >/dev/null 2>&1
[ "$closed_before" = "$(cat "$tmp/repo/knowledge/open-questions-closed.md")" ] \
  && ok "повторный прогон архив не меняет" || bad "повторный прогон архив не меняет" "архив без изменений" "$(cat "$tmp/repo/knowledge/open-questions-closed.md")"

# Путь из ответов: архив рядом с файлом вопросов, а не в knowledge/.
setup docs/questions.md
perl -i -pe 's/^## Второй вопрос$/## ~~Второй вопрос~~ — закрыт 08.10.2026/' "$tmp/repo/docs/questions.md"
"$tmp/repo/knowledge/archive-questions.sh" >/dev/null 2>&1
grep -q '~~Второй вопрос~~' "$tmp/repo/docs/questions-closed.md" 2>/dev/null \
  && ok "архив рядом с файлом из ответов" || bad "архив рядом с файлом из ответов" "docs/questions-closed.md" "$(ls "$tmp/repo/docs")"

# --stage: оба файла в индексе.
setup knowledge/open-questions.md
git -C "$tmp/repo" init -q
git -C "$tmp/repo" add -A
git -C "$tmp/repo" -c user.email=t@t -c user.name=t commit -q -m base
perl -i -pe 's/^## Первый вопрос$/## ~~Первый вопрос~~ — закрыт 08.10.2026/' "$tmp/repo/knowledge/open-questions.md"
(cd "$tmp/repo" && ./knowledge/archive-questions.sh --stage >/dev/null 2>&1)
staged=$(git -C "$tmp/repo" diff --cached --name-only | sort | tr '\n' ' ')
[ "$staged" = "knowledge/open-questions-closed.md knowledge/open-questions.md " ] \
  && ok "--stage ставит в индекс оба файла" || bad "--stage ставит в индекс оба файла" "оба файла" "$staged"

out=$("$tmp/repo/knowledge/archive-questions.sh" --что-то 2>&1); code=$?
[ $code -eq 2 ] && ok "непонятный аргумент" || bad "непонятный аргумент" "код 2" "код $code: $out"

echo
printf 'итого: %d прошло, %d провалено\n' "$pass" "$fail"
[ "$fail" -eq 0 ]
