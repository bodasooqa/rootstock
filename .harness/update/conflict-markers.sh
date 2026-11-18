#!/usr/bin/env bash
# Merge conflict markers in project files. Called by the `markers` job in
# `.github/workflows/harness.yml`: a harness update conflict (`rootstock
# update`) stays as markers in files, and CI must go red on it.
#
# A conflict is three lines, in order, at the start of a line: `<<<<<<< ` with a
# label, `=======` alone, `>>>>>>> ` with a label (`||||||| ` may sit between the
# first and the second, diff3 style). A lone line of seven `<` or `=` (a frame
# in output, a heading underline) is not a conflict, nor is a marker in the
# middle of a line (in quotes, in code).
#
# Searches tracked and untracked non-ignored files of the repository, skipping
# binary ones.
#
# Prints the files with a conflict and exits with code 1; none, code 0.
set -euo pipefail

cd "$(git rev-parse --show-toplevel)"

found=0
while IFS= read -r -d '' file; do
  if awk '
    /^<<<<<<< /                 { state = 1; next }
    state == 1 && /^=======\r?$/ { state = 2; next }
    state == 2 && /^>>>>>>> /   { hit = 1; exit }
    END                         { exit !hit }
  ' "$file"; then
    printf '%s\n' "$file"
    found=1
  fi
done < <(git grep -I -l -z --untracked -E '^<<<<<<< ' || true)

if ((found)); then
  echo "conflict-markers.sh: the files above contain merge conflict markers; resolve the conflict and commit" >&2
fi
exit "$found"
