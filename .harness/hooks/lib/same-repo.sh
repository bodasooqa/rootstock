#!/usr/bin/env bash
# Whose file is being edited: checks that execute a worktree's code (`tsc` from its
# node_modules, `cargo` with its build.rs, macros and rust-toolchain.toml) run
# only for our own repository — this worktree or a neighbouring worktree of the
# same repository. Editing a file in a foreign clone is no reason to run its code.
#
# Ours means three conditions, and none of them asks git in the checked directory:
# its answer is set by the directory itself (`core.worktree` in the config, a
# planted `.git`), and it would name the project worktree as the root while the
# run happens in a foreign directory.
#   - the physical path lies in a worktree from the project's `git worktree list`;
#   - there is no `.git` between the directory and the worktree root — otherwise
#     it is a nested clone or a planted git file;
#   - the directory is not ignored in the worktree (`tmp/`, `node_modules/`,
#     an unpacked foreign package): it lies in the worktree but doesn't belong
#     to the project.
# An untracked but not ignored directory counts as ours: a new package before
# its first commit looks the same.

# same_repo <dir> — succeeds if the directory is in a registered project worktree.
same_repo() {
  local project=${CLAUDE_PROJECT_DIR:-$PWD} dir tree d
  dir=$(cd "$1" 2>/dev/null && pwd -P) || return 1
  while IFS= read -r tree; do
    tree=$(cd "$tree" 2>/dev/null && pwd -P) || continue
    case "$dir/" in
      "$tree"/*) ;;
      *) continue ;;
    esac
    # Worktrees can be nested in one another: a `.git` on the way to the outer
    # one is the root of the inner one, and its turn to be checked will come.
    d=$dir
    while [ "$d" != "$tree" ] && [ ! -e "$d/.git" ]; do
      d=${d%/*}
    done
    [ "$d" = "$tree" ] || continue
    git -C "$tree" check-ignore -q -- "$dir" 2>/dev/null && return 1
    return 0
  done < <(git -C "$project" worktree list --porcelain 2>/dev/null | sed -n 's/^worktree //p')
  return 1
}
