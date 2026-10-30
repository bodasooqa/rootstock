#!/bin/sh
# Render parity with Copier: every valid combination of the answers rust,
# frontend, monorepo, linear, telegram, orca and doc_form of harness <tag>,
# rendered by Copier and by rootstock (the same tag converted in the working
# tree, `new --version HEAD`), compared tree by tree. See README.md.
#
# Usage: render.sh <rootstock-binary> <tag>
set -u
[ $# -eq 2 ] || { echo "usage: $0 <rootstock-binary> <tag>" >&2; exit 1; }
BIN=$(cd "$(dirname "$1")" && pwd)/$(basename "$1"); TAG=$2
HERE=$(cd "$(dirname "$0")" && pwd)
# shellcheck source=tests/parity/env.sh
. "$HERE/env.sh"

for side in copier rootstock; do
  git clone -q --branch "$TAG" "$HARNESS" "$WORK/$side-src" || exit 1
done
(cd "$WORK/rootstock-src" && "$BIN" import-copier >/dev/null) || exit 1
COMMIT=$(git -C "$WORK/copier-src" rev-parse HEAD)

# Valid combinations: `monorepo` is asked only when not both rust and
# frontend, `telegram` only with rust; "-" means the question is not asked.
combos() {
  for rf in true,true true,false false,true; do
    r=${rf%,*}; f=${rf#*,}
    if [ "$r" = true ] && [ "$f" = true ]; then monos=-; else monos="true false"; fi
    if [ "$r" = true ]; then tgs="true false"; else tgs=-; fi
    for m in $monos; do for t in $tgs; do
      for l in true false; do for o in true false; do for d in catalog monolith; do
        echo "$r $f $m $t $l $o $d"
      done; done; done
    done; done
  done
}

n=0; failed=0
combos > "$WORK/combos"
while read -r r f m t l o d; do
  n=$((n + 1))
  name="rust=$r,frontend=$f,monorepo=$m,telegram=$t,linear=$l,orca=$o,doc_form=$d"
  set -- --data project_name=demo --data rust="$r" --data frontend="$f" \
    --data linear="$l" --data orca="$o" --data doc_form="$d"
  [ "$m" != - ] && set -- "$@" --data monorepo="$m"
  [ "$t" != - ] && set -- "$@" --data telegram="$t"
  [ "$l" = true ] && set -- "$@" --data tracker_prefix=DEN
  [ "$d" = monolith ] && set -- "$@" --data spec_path=docs/design.md \
    --data revisions_path=docs/revisions.md --data decisions_path=knowledge/decisions.md \
    --data lessons_path=knowledge/lessons.md --data open_questions_path=knowledge/open-questions.md \
    --data stages_path=knowledge/stages.md
  c="$WORK/$n-copier"; rs="$WORK/$n-rootstock"
  # A bare PATH keeps both tools' tasks identical (no openspec on PATH).
  if ! PATH=/usr/bin:/bin "$COPIER" copy -q --trust --defaults --vcs-ref "$TAG" "$@" \
    "$WORK/copier-src" "$c" >"$c.log" 2>&1; then
    echo "$name: copier failed, see $c.log"; failed=$((failed + 1)); continue
  fi
  if ! PATH=/usr/bin:/bin "$BIN" new "$WORK/rootstock-src" "$rs" --version HEAD \
    --defaults --trust "$@" >"$rs.log" 2>&1; then
    echo "$name: rootstock failed, see $rs.log"; failed=$((failed + 1)); continue
  fi
  diffs=$("$PY" -I "$HERE/compare.py" trees "$c" "$rs";
    "$PY" -I "$HERE/compare.py" answers "$c/.copier-answers.yml" "$rs/.rootstock.toml" "$COMMIT")
  if [ -n "$diffs" ]; then
    echo "== $name"; echo "$diffs"; failed=$((failed + 1))
  fi
done < "$WORK/combos"
echo "$n combinations, $failed with differences"
[ "$failed" -eq 0 ]
