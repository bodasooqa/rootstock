#!/usr/bin/env bash
# Tests for `knowledge/due.sh`. A check guarding a gate must be checked
# itself: a gate that silently finds nothing is worse than no gate.
#
# Every case covers both firing and passing: overdue is found where it exists
# and is not invented where it does not. The project is synthetic, in a
# temporary directory: its own answers file, stage table and question list.
#
# Run: knowledge/tests/due.sh
set -uo pipefail

here=$(cd "${BASH_SOURCE[0]%/*}" && pwd)
script="$here/../due.sh"

pass=0
fail=0
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

ok()  { pass=$((pass + 1)); printf '  ok   %s\n' "$1"; }
bad() { fail=$((fail + 1)); printf '  FAIL %s\n     expected: %s\n     got: %s\n' "$1" "$2" "${3:-<empty>}"; }

# A separate project per case. Paths of the questions and the stage table are
# arguments, catalog-form defaults; the question list comes from stdin.
setup() {
  local questions=${1:-knowledge/open-questions.md} stages=${2:-docs/spec/03-stages.md}
  rm -rf "$tmp/repo"
  mkdir -p "$tmp/repo/knowledge" "$tmp/repo/openspec/changes/archive" \
    "$tmp/repo/$(dirname "$questions")" "$tmp/repo/$(dirname "$stages")"
  cp "$script" "$tmp/repo/knowledge/due.sh"
  printf '[answers]\nopen_questions_path = "%s"\nstages_path = "%s"\n' "$questions" "$stages" > "$tmp/repo/.rootstock.toml"
  cat > "$tmp/repo/$stages" <<'MD'
# Stages

| Stage | State |
|---|---|
| 1. Skeleton | closed |
| 2. Storage | closed |
| 3. API | in progress |
| 4–8. The rest | not started |
MD
  {
    printf '# Open questions\n\n'
    cat
  } > "$tmp/repo/$questions"
}

run() {
  local out code
  out=$("$tmp/repo/knowledge/due.sh" "$@" 2>&1)
  code=$?
  printf '%s\n%s' "$code" "$out"
}

# check <name> <expected code|!0> <expected substring|-> [due.sh arguments]
check() {
  local name=$1 want_code=$2 want_text=$3
  shift 3
  local res code out
  res=$(run "$@")
  code=${res%%$'\n'*}
  out=${res#*$'\n'}
  if [ "$want_code" = "!0" ] && [ "$code" != 0 ]; then
    :
  elif [ "$code" != "$want_code" ]; then
    bad "$name" "exit $want_code" "exit $code: $out"
    return
  fi
  if [ "$want_text" != "-" ] && [[ "$out" != *"$want_text"* ]]; then
    bad "$name" "\"$want_text\" in output" "$out"
    return
  fi
  ok "$name"
}

echo "due.sh --overdue"

setup <<'MD'
## Question for a closed stage

Gist.

Due: stage 2, where the storage format will need a decision anyway.
MD
check "question for a closed stage — overdue" 1 "Question for a closed stage" --overdue

setup <<'MD'
## Question for a stage ahead

Gist.

Due: stage 4, where a second client appears.
MD
check "question for a future stage — not overdue" 0 "No overdue questions" --overdue

setup <<'MD'
## Question for the stage in progress

Gist.

Due: stage 3, where the API appears.
MD
check "question for the current stage — not overdue" 0 "No overdue questions" --overdue

setup <<'MD'
## Question without a deadline

There is a gist, but no "Due" line.
MD
check "question without a deadline — overdue" 1 'no "Due:" line' --overdue

setup <<'MD'
## Question with a prose deadline

Gist.

Due: the stage where there is more than one user.
MD
check "prose deadline — overdue" 1 "deadline does not name a stage" --overdue

setup <<'MD'
## ~~Closed question~~ — closed 2026-09-10

Gist.

Due: stage 1.
MD
check "struck-through question is not counted" 0 "No overdue questions" --overdue

setup <<'MD'
## Question for an archived change

Gist.

Due: `stage3-public-api`, where this becomes visible.
MD
mkdir -p "$tmp/repo/openspec/changes/archive/2026-09-10-stage3-public-api"
check "change archived — overdue" 1 "is archived" --overdue

setup <<'MD'
## Question for a live change

Gist.

Due: `stage3-public-api`, where this becomes visible.
MD
check "change not archived — not overdue" 0 "No overdue questions" --overdue

# A stage number from the second sentence is not a deadline: the deadline is the first.
setup <<'MD'
## Deadline in one sentence, stage number in another

Gist.

Due: the stage where there are more than a thousand records. Since stage 2 the question is moot.
MD
check "stage number from the next sentence is not taken" 1 "deadline does not name a stage" --overdue

# A format example in a code block is not a question, even with "Due:" without a number.
setup <<'MD'
Format:

```markdown
## Question title

Due: stage N.
```

---
MD
check "format example in a code block is not counted" 0 "No overdue questions" --overdue

echo
echo "due.sh — paths from the answers file"

# Monolith form: questions and stage table live where the answers point.
setup docs/questions.md README.md <<'MD'
## Question for a closed stage

Gist.

Due: stage 1.
MD
check "questions path from the answers in the report" 1 "docs/questions.md:" --overdue

setup <<'MD'
## Question

Due: stage 3.
MD
printf 'stages_path = "docs/spec/03-stages.md"\n\n[answers]\nopen_questions_path = "knowledge/open-questions.md"\n' > "$tmp/repo/.rootstock.toml"
check "no stages_path in the answers (outside [answers] does not count) — error" 1 "has no key stages_path" --overdue

setup <<'MD'
## Question

Due: stage 3.
MD
printf '# Stages\n\nNo table.\n' > "$tmp/repo/docs/spec/03-stages.md"
check "no stage table — error" '!0' "no stage state table found" --overdue

echo
echo "due.sh — report"

setup <<'MD'
## Question for the stage in progress

Gist.

Due: stage 3, where the API appears.
MD
check "without an argument the stage in progress is taken" 0 "Open questions for stage 3"
check "stage number as an argument"                       0 "Open questions for stage 4" 4
check "no questions for the named stage"                  0 "No questions are addressed to this stage" 4
check "unknown argument"                                  2 "unknown argument" --something

echo
printf 'total: %d passed, %d failed\n' "$pass" "$fail"
[ "$fail" -eq 0 ]
