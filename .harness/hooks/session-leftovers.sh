#!/usr/bin/env bash
# Session leftovers: what was left unfinished in this worktree.
#
# The rule "finish the change" was held only by a skill, and leftovers pile up
# unnoticed: in one day a session left two worktrees, three branches on origin
# and a main checkout two PRs behind main. None of it is visible until you go
# and look by hand.
#
# The hook blocks nothing and deletes nothing: it only names. Slot
# `session-end` (a report to the human on exit); it is also mixed into the
# start report of `session-tree.sh` (a report to the model at the start of the
# next session).
#
# `Stop` doesn't fit: it can bring text to the model only through
# `decision: "block"`, i.e. by forcing the session to continue its turn.
# Uncommitted edits are a state kept on purpose, and such a hook would block
# every exit.
#
# Prints report lines or nothing if there are no leftovers. Exit code is always 0.
set -uo pipefail

git rev-parse --is-inside-work-tree >/dev/null 2>&1 || exit 0

root=$(git rev-parse --show-toplevel 2>/dev/null) || exit 0
branch=$(git rev-parse --abbrev-ref HEAD 2>/dev/null)
[ -n "$branch" ] || exit 0

lines=()

# --- Uncommitted edits ---------------------------------------------------------
# `node_modules` is symlinked into every worktree by the `worktree` setting and
# is not the session's work — otherwise every worktree looks dirty from the first second.
dirty_paths() {
  git -C "${1:-.}" status --porcelain 2>/dev/null | grep -v '^?? node_modules/\?$'
}

dirty=$(dirty_paths . | grep -c . || true)
if [ "${dirty:-0}" -gt 0 ]; then
  lines+=("uncommitted paths: $dirty (git status)")
fi

# --- Unpushed commits ----------------------------------------------------------
# A branch without an upstream is a leftover too: nobody has seen it yet.
if upstream=$(git rev-parse --abbrev-ref --symbolic-full-name '@{upstream}' 2>/dev/null); then
  ahead=$(git rev-list --count "$upstream..HEAD" 2>/dev/null || echo 0)
  if [ "${ahead:-0}" -gt 0 ]; then
    lines+=("unpushed commits: $ahead (branch $branch, upstream $upstream)")
  fi
else
  base=$(git rev-parse --verify --quiet origin/main >/dev/null 2>&1 && echo origin/main || echo main)
  own=$(git rev-list --count "$base..HEAD" 2>/dev/null || echo 0)
  if [ "${own:-0}" -gt 0 ]; then
    lines+=("branch $branch has never been pushed, own commits: $own")
  fi
fi

# --- Worktrees whose work is already in main -----------------------------------
# A worktree counts as merged when its commits don't differ from main in content:
# PRs here are squash-merged, so ancestry is not a sign.
if git rev-parse --verify --quiet origin/main >/dev/null 2>&1; then
  main_ref=origin/main
else
  main_ref=main
fi

merged=$(git worktree list --porcelain 2>/dev/null | awk '
  /^worktree /{ wt = substr($0, 10) }
  /^branch /{ br = substr($0, 8); sub(/^refs\/heads\//, "", br); print wt "\t" br }
' | while IFS=$'\t' read -r wt br; do
  [ "$br" = "main" ] && continue
  # A worktree with uncommitted edits is left alone and kept out of the report:
  # it is busy, not forgotten.
  [ -n "$(dirty_paths "$wt")" ] && continue
  ahead=$(git rev-list --count "$main_ref..$br" 2>/dev/null || echo 0)
  [ "${ahead:-0}" -eq 0 ] && continue
  # Two signs "the work is already there": the branch is an ancestor of main (a
  # regular merge) or its content matches main (a squash merge, as in this repository).
  # The second sign stops working as soon as main moves ahead, so the hook doesn't
  # promise to find all merged worktrees — only those visible for certain.
  if git merge-base --is-ancestor "$br" "$main_ref" 2>/dev/null \
    || [ -z "$(git diff --stat "$br" "$main_ref" 2>/dev/null)" ]; then
    printf '  %s (%s)\n' "$wt" "$br"
  fi
done)

if [ -n "$merged" ]; then
  lines+=("worktrees whose work is already in $main_ref — can be removed:")
  lines+=("$merged")
fi

# --- Divergences ---------------------------------------------------------------
# `merge-tree --write-tree` plays the merge out dry: the working tree is not
# touched, the index is not touched, the answer comes in a fraction of a second.
# The network is not touched either, so the comparison is against the `origin/main`
# available locally.
#
# Added 10.09.2026: two PRs diverged on append-only journals, and it was
# discovered when both were already open. The divergence existed long before
# the merge — nobody just asked about it.
conflicting_files() {
  git merge-tree --write-tree "$1" "$2" 2>/dev/null \
    | sed -n 's/^CONFLICT ([^)]*): Merge conflict in //p'
}

if [ "$branch" != "main" ] && git rev-parse --verify --quiet "$main_ref" >/dev/null 2>&1; then
  with_main=$(conflicting_files "$main_ref" HEAD)
  if [ -n "$with_main" ]; then
    lines+=("diverges from $main_ref in files:")
    lines+=("$(printf '  %s\n' $with_main)")
  fi
fi

# Neighbouring worktrees: the divergence is visible before the other branch reaches main.
neighbours=$(git worktree list --porcelain 2>/dev/null | awk -v self="$branch" '
  /^branch /{ br = substr($0, 8); sub(/^refs\/heads\//, "", br)
              if (br != self && br != "main") print br }
')

for br in ${neighbours:-}; do
  files=$(conflicting_files HEAD "$br")
  [ -z "$files" ] && continue
  lines+=("will diverge from branch $br in files:")
  lines+=("$(printf '  %s\n' $files)")
done

[ ${#lines[@]} -eq 0 ] && exit 0

printf 'Session leftovers in %s:\n' "$root"
for l in "${lines[@]}"; do
  case "$l" in
    "  "*) printf '%s\n' "$l" ;;
    *) printf -- '- %s\n' "$l" ;;
  esac
done
printf 'To finish up — the change-finish skill.\n'
exit 0
