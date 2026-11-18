#!/usr/bin/env bash
# Tests for `knowledge/archive-questions.sh`: a closed question goes to the
# archive, an open one stays; with nothing closed the files are not touched;
# `--stage` stages both files. The project is synthetic, in a temporary
# directory.
#
# Run: knowledge/tests/archive-questions.sh
set -uo pipefail

# Under a git hook the environment carries GIT_DIR and friends: git would look
# at the hook's repository instead of the temporary one.
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
bad() { fail=$((fail + 1)); printf '  FAIL %s\n     expected: %s\n     got: %s\n' "$1" "$2" "${3:-<empty>}"; }

# setup <questions path> — a project with an answers file and two questions.
setup() {
  local questions=$1
  rm -rf "$tmp/repo"
  mkdir -p "$tmp/repo/knowledge" "$tmp/repo/$(dirname "$questions")"
  cp "$script" "$tmp/repo/knowledge/"
  printf '[answers]\nopen_questions_path = "%s"\n' "$questions" > "$tmp/repo/.rootstock.toml"
  printf '# Open questions\n\nHead.\n\n---\n\n## First question\n\nGist.\n\n---\n\n## Second question\n\nGist.\n' \
    > "$tmp/repo/$questions"
}

echo "archive-questions.sh"

setup knowledge/open-questions.md
before=$(cat "$tmp/repo/knowledge/open-questions.md")
out=$("$tmp/repo/knowledge/archive-questions.sh" 2>&1); code=$?
{ [ $code -eq 0 ] && [ -z "$out" ] && [ "$before" = "$(cat "$tmp/repo/knowledge/open-questions.md")" ] \
  && [ ! -e "$tmp/repo/knowledge/open-questions-closed.md" ]; } \
  && ok "nothing closed — nothing touched" || bad "nothing closed — nothing touched" "silence, files in place" "exit $code: $out"

perl -i -pe 's/^## First question$/## ~~First question~~ — closed 2026-10-08/' "$tmp/repo/knowledge/open-questions.md"
out=$("$tmp/repo/knowledge/archive-questions.sh" 2>&1); code=$?
open=$(cat "$tmp/repo/knowledge/open-questions.md")
closed=$(cat "$tmp/repo/knowledge/open-questions-closed.md" 2>/dev/null)
{ [ $code -eq 0 ] && [[ "$open" != *"First question"* ]] && [[ "$open" == *"## Second question"* ]] \
  && [[ "$closed" == *"# Closed questions"* ]] \
  && [[ "$closed" == *"## ~~First question~~"* ]] && [[ "$closed" != *"Second question"* ]]; } \
  && ok "closed goes to the archive, open stays" \
  || bad "closed goes to the archive, open stays" "First in the archive, Second in place" "exit $code: $out / $open / $closed"

# A second run: nothing to move, the archive does not grow.
closed_before=$closed
"$tmp/repo/knowledge/archive-questions.sh" >/dev/null 2>&1
[ "$closed_before" = "$(cat "$tmp/repo/knowledge/open-questions-closed.md")" ] \
  && ok "a second run does not change the archive" || bad "a second run does not change the archive" "archive unchanged" "$(cat "$tmp/repo/knowledge/open-questions-closed.md")"

# Path from the answers: the archive sits next to the questions file, not in knowledge/.
setup docs/questions.md
perl -i -pe 's/^## Second question$/## ~~Second question~~ — closed 2026-10-08/' "$tmp/repo/docs/questions.md"
"$tmp/repo/knowledge/archive-questions.sh" >/dev/null 2>&1
grep -q '~~Second question~~' "$tmp/repo/docs/questions-closed.md" 2>/dev/null \
  && ok "archive next to the file from the answers" || bad "archive next to the file from the answers" "docs/questions-closed.md" "$(ls "$tmp/repo/docs")"

# --stage: both files staged.
setup knowledge/open-questions.md
git -C "$tmp/repo" init -q
git -C "$tmp/repo" add -A
git -C "$tmp/repo" -c user.email=t@t -c user.name=t commit -q -m base
perl -i -pe 's/^## First question$/## ~~First question~~ — closed 2026-10-08/' "$tmp/repo/knowledge/open-questions.md"
(cd "$tmp/repo" && ./knowledge/archive-questions.sh --stage >/dev/null 2>&1)
staged=$(git -C "$tmp/repo" diff --cached --name-only | sort | tr '\n' ' ')
[ "$staged" = "knowledge/open-questions-closed.md knowledge/open-questions.md " ] \
  && ok "--stage stages both files" || bad "--stage stages both files" "both files" "$staged"

out=$("$tmp/repo/knowledge/archive-questions.sh" --something 2>&1); code=$?
[ $code -eq 2 ] && ok "unknown argument" || bad "unknown argument" "exit 2" "exit $code: $out"

echo
printf 'total: %d passed, %d failed\n' "$pass" "$fail"
[ "$fail" -eq 0 ]
