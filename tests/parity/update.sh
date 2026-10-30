#!/bin/sh
# Update parity with Copier: a copy of a Copier-generated project at
# <commit> is updated to harness <tag> twice, by `copier update` and by
# `rootstock import-copier` + `rootstock update`, and the trees are compared,
# Copier's conflict labels rewritten to rootstock's. See README.md.
#
# Usage: update.sh <rootstock-binary> <project-commit> <tag>
set -u
[ $# -eq 3 ] || { echo "usage: $0 <rootstock-binary> <project-commit> <tag>" >&2; exit 1; }
BIN=$(cd "$(dirname "$1")" && pwd)/$(basename "$1"); BASE=$2; TAG=$3
HERE=$(cd "$(dirname "$0")" && pwd)
# shellcheck source=tests/parity/env.sh
. "$HERE/env.sh"
SCREENER=${SCREENER:-https://github.com/bodasooqa/screener}

git clone -q "$HARNESS" "$WORK/harness" || exit 1
for side in copier rootstock; do
  git clone -q "$SCREENER" "$WORK/$side" || exit 1
  # The template is taken from the local clone, at the recorded tags.
  (cd "$WORK/$side" && git switch -q -c parity "$BASE" \
    && sed "s|^_src_path: .*|_src_path: $WORK/harness|" .copier-answers.yml > .answers.tmp \
    && mv .answers.tmp .copier-answers.yml \
    && git commit -qam "parity: local template") || exit 1
done

(cd "$WORK/copier" && PATH=/usr/bin:/bin "$COPIER" update -q --trust --defaults \
  --vcs-ref "$TAG" >"$WORK/copier.log" 2>&1)
code=$?; echo "copier update: exit $code"
[ $code -eq 0 ] || { echo "copier failed, see $WORK/copier.log"; exit 1; }
(cd "$WORK/rootstock" && "$BIN" import-copier >"$WORK/import.log" 2>&1)
code=$?; echo "rootstock import-copier: exit $code"
[ $code -eq 0 ] || { echo "import-copier failed, see $WORK/import.log"; exit 1; }
(cd "$WORK/rootstock" && git add -A && git commit -qm "parity: import" \
  && PATH=/usr/bin:/bin "$BIN" update --to "$TAG" --trust >"$WORK/update.log" 2>&1)
code=$?; echo "rootstock update: exit $code ($(tr '\n' ';' < "$WORK/update.log"))"
[ $code -eq 0 ] || [ $code -eq 2 ] || { echo "rootstock update failed"; exit 1; }

COMMIT=$(git -C "$WORK/harness" rev-parse "$TAG^{commit}")
"$PY" -I "$HERE/compare.py" trees "$WORK/copier" "$WORK/rootstock" --copier-markers "$TAG"
trees=$?
"$PY" -I "$HERE/compare.py" answers "$WORK/copier/.copier-answers.yml" \
  "$WORK/rootstock/.rootstock.toml" "$COMMIT"
answers=$?
[ $trees -eq 0 ] && [ $answers -eq 0 ] || exit 1
echo "no differences"
