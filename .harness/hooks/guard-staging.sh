#!/usr/bin/env bash
# Slot pre-bash (PreToolUse/Bash): forbids wholesale staging in the shared
# checkout.
#
# Two sessions shared one index, and one session's `git add` carried the other's
# files into its commit. A session's own worktree has its own index, there is
# nothing to carry off — we don't interfere there.
set -uo pipefail

. "${BASH_SOURCE[0]%/*}/lib/bash-scan.sh"

# Linked worktree: the session's git dir differs from the common one.
gitdir=$(git rev-parse --git-dir 2>/dev/null) || exit 0
common=$(git rev-parse --git-common-dir 2>/dev/null) || exit 0
[ "$gitdir" = "$common" ] || exit 0

scan=$(bash_scan) || exit 0

# Quoted content is data. Otherwise `git commit -m "add -a"` reads as a flag.
scan=$(printf '%s' "$scan" | sed -E 's/"[^"]*"/""/g; s/'\''[^'\'']*'\''/'\'\''/g')

wholesale="In the shared checkout the index is shared by all sessions: wholesale staging will carry other sessions' files into the commit. Stage named paths: git add <file> [<file>…]. Need your whole set at once — create a session worktree (orca worktree create), it has its own index."

if printf '%s' "$scan" | grep -Eq '(^|[;&|[:space:]])git[[:space:]]+add([[:space:]]+-[^;&|[:space:]]+)*[[:space:]]+(\.|\*|:/)([[:space:]]|$)'; then
  deny "$wholesale"
fi

if printf '%s' "$scan" | grep -Eq '(^|[;&|[:space:]])git[[:space:]]+add[^;&|]*[[:space:]](-A|--all|-u|--update)([[:space:]]|$)'; then
  deny "$wholesale"
fi

if printf '%s' "$scan" | grep -Eq '(^|[;&|[:space:]])git[[:space:]]+commit[^;&|]*[[:space:]](-[a-zA-Z]*a[a-zA-Z]*|--all)([[:space:]]|$)'; then
  deny "git commit -a commits everything changed in the shared checkout, including neighbouring sessions' edits. Stage the named files and commit without -a."
fi

exit 0
