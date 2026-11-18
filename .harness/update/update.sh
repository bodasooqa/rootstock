#!/usr/bin/env bash
# Updates the project to a harness version via rootstock. Called from
# `.github/workflows/harness-update.yml`; the logic lives here so it can be
# tested without GitHub (`.harness/update/tests/update.sh`).
#
# Usage:
#   update.sh <tag>
#       Updates the project in the current directory to the tag (`rootstock update`)
#       and commits it as a single commit "Update harness to <tag>". Prints
#       conflicts one per line, `<kind> <path>`, like rootstock; exit code 0
#       even with conflicts. The commit identity comes from the environment
#       (GIT_AUTHOR_*, GIT_COMMITTER_*).
#
# The rootstock command is the `ROOTSTOCK` environment variable, as a string
# (default `rootstock`). The workflow installs a pinned version.
set -euo pipefail

[[ $# -eq 1 ]] || {
  printf 'update.sh: usage: update.sh <tag>\n' >&2
  exit 2
}
next=$1

cd "$(git rev-parse --show-toplevel)"

found=$(mktemp)
trap 'rm -f "$found"' EXIT
code=0
# shellcheck disable=SC2086 # the command is a string, with its own arguments
${ROOTSTOCK:-rootstock} update --to "$next" --defaults --trust >"$found" || code=$?
# Code 2 means the update succeeded with conflicts: not an error, conflicts go to the PR.
case $code in
  0 | 2) ;;
  *)
    printf 'update.sh: rootstock update exited with code %s\n' "$code" >&2
    exit 1
    ;;
esac
git add -A
git commit --quiet --no-verify -m "Update harness to $next"
sed '/^$/d' "$found" | sort -u
