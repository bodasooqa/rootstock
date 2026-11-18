#!/usr/bin/env bash
# Slot session-start (SessionStart): reports where the session works and who else
# is busy in the repository.
#
# The rule "one worktree per session" was held only by CLAUDE.md prose, and the
# price of breaking it is other sessions' files in your commit. The session must
# learn the situation before the first edit, not after someone else's commit. The
# hook looks only at `git worktree` and calls no worktree managers.
set -uo pipefail

git rev-parse --is-inside-work-tree >/dev/null 2>&1 || exit 0

root=$(git rev-parse --show-toplevel 2>/dev/null) || exit 0
gitdir=$(git rev-parse --git-dir 2>/dev/null)
common=$(git rev-parse --git-common-dir 2>/dev/null)
branch=$(git rev-parse --abbrev-ref HEAD 2>/dev/null)

if [ "$gitdir" = "$common" ]; then
  where="The session works in the SHARED checkout $root (branch $branch). The index here is shared by all sessions."
  own="no"
else
  where="The session works in its own worktree $root (branch $branch)."
  own="yes"
fi

# Other worktrees: everything from git worktree list except the current one.
others=$(git worktree list --porcelain 2>/dev/null | awk -v self="$root" '
  /^worktree /{ wt = substr($0, 10) }
  /^branch /{ br = substr($0, 8); sub(/^refs\/heads\//, "", br)
              if (wt != self) printf "  %s (%s)\n", wt, br }
')

note="$where"
if [ -n "$others" ]; then
  note="$note"$'\n'"Other worktrees of this repository:"$'\n'"$others"
fi

# A build directory shared by all worktrees doesn't work: the package is named
# the same in every worktree while the code differs, and one worktree's build
# lands on top of another's. The config is machine-specific and not committed,
# so it won't go away by itself — on a machine where it remains, a green run in
# a worktree proves nothing.
common_abs=$(cd "$common" 2>/dev/null && pwd)
if [ -n "$common_abs" ]; then
  cargo_config="$(dirname "$common_abs")/.cargo/config.toml"
  if grep -qs '^[[:space:]]*target-dir' "$cargo_config"; then
    note="$note"$'\n'"A shared build directory is configured: $cargo_config sets target-dir. Worktrees with a shared directory overwrite each other's builds. To remove: rm -f $cargo_config."
  fi
fi

# Git hooks are shared by all worktrees, and lefthook writes the absolute path to
# its binary into them. Installing from a worktree (the lefthook package's
# postinstall on `bun install`) points the path into that worktree's node_modules;
# the worktree is removed — and pre-commit and pre-push silently stop working in
# the whole repository. A path into node_modules must lead to the main checkout;
# a path outside node_modules (lefthook installed globally) must exist.
main_dir=$(cd "$common/.." 2>/dev/null && pwd -P)
if [ -n "$main_dir" ] && [ -f "$main_dir/lefthook.yml" ]; then
  hooks_dir=$(git rev-parse --path-format=absolute --git-path hooks 2>/dev/null)
  broken=""
  for h in pre-commit pre-push; do
    hook="$hooks_dir/$h"
    bin=$(sed -n 's|^[[:space:]]*elif \(/[^ ]*/lefthook\) -h.*|\1|p' "$hook" 2>/dev/null | head -n 1)
    case "$bin" in
      "") broken="$broken"$'\n'"  $hook — no path to lefthook" ;;
      "$main_dir/node_modules/"*) [ -x "$bin" ] || broken="$broken"$'\n'"  $hook → $bin — file missing" ;;
      */node_modules/*) broken="$broken"$'\n'"  $hook → $bin — not the main checkout" ;;
      *) [ -x "$bin" ] || broken="$broken"$'\n'"  $hook → $bin — file missing" ;;
    esac
  done
  if [ -n "$broken" ]; then
    note="$note"$'\n'"Git hooks are broken, pre-commit and pre-push don't run:$broken"$'\n'"Fix: cd $main_dir && lefthook install"
  fi
fi

# Overdue open questions. One line, no listing: `knowledge/due.sh` prints the
# list, and the report only keeps the debt from going unnoticed.
# The script comes from the hook's project, as in knowledge-due.sh, not from the
# worktree of the session's directory: a session started in a foreign clone
# doesn't execute its code.
due="$(cd "${BASH_SOURCE[0]%/*}/../.." 2>/dev/null && pwd)/knowledge/due.sh"
if [ -x "$due" ]; then
  overdue=$("$due" --overdue 2>/dev/null | sed -n 's/^Overdue questions: \([0-9]*\)$/\1/p')
  if [ -n "$overdue" ]; then
    note="$note"$'\n'"Overdue open questions: $overdue. List — knowledge/due.sh --overdue, triage — the stage-grooming skill."
  fi
fi

# Leftovers of the previous session: the same text that goes to the human on
# exit, only here it reaches the model on entry — tidying up is easier at the
# start than at the end.
leftovers="${BASH_SOURCE[0]%/*}/session-leftovers.sh"
if [ -x "$leftovers" ]; then
  tails=$("$leftovers" 2>/dev/null)
  [ -n "$tails" ] && note="$note"$'\n'"$tails"
fi

if [ "$own" = "no" ]; then
  note="$note"$'\n'"Editing files from the shared checkout is allowed only when there are certainly no other sessions on this repository. Otherwise create your own worktree: orca worktree create. Wholesale staging is forbidden here by a hook — stage named paths."
fi

jq -nc --arg n "$note" '{
  hookSpecificOutput: { hookEventName: "SessionStart", additionalContext: $n }
}'
exit 0
