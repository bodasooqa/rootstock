#!/usr/bin/env bash
# The third fuse of the asynchronous checks: a verdict that is stale by the time
# it is ready doesn't wake anyone.
#
# The lock and the error signature (`rust-clippy.sh`) suppress a repeat of the
# same cause, but are powerless against a refactoring edit: there the cause is NEW
# at every step — rename a type, and one call site breaks after another.
# The signature honestly changes and honestly wakes, and for one screen of edits
# the session got two dozen wake-ups in a row, none of which was about the state
# on disk.
#
# The point is not frequency but address: an asynchronous check takes seconds
# (`tsc`) or minutes (`clippy`), and a batch of edits moves ahead meanwhile. Waking
# with text about a state that no longer exists is not a report but noise: there
# is nothing in it to fix. An intermediate red state of a TDD refactoring is red
# by construction, and whoever created it fixes it.
#
# The fuse can't stay silent forever: while edits continue, each one restarts
# the check, and the last edit of the batch has no next one — its verdict is
# fresh, and it reports. So the silence lasts exactly as long as the batch.

# stale_marker — prints the path to a freshly created check-start timestamp.
# An empty string means the marker couldn't be created; the caller then works
# the old way, without the fuse, rather than staying silent.
stale_marker() {
  mktemp "${TMPDIR:-/tmp}/harness-hook-mark.XXXXXX" 2>/dev/null || printf ''
}

# stale_since <marker> <dir> [excluded-file] — exit 0 if a file newer than the
# marker appeared in the directory, i.e. the worktree moved on while the check ran.
#
# The excluded file is the one whose edit triggered the hook: the synchronous
# `rust-fmt.sh` and `js-fmt.sh` rewrite exactly it and may do so after the marker.
# Without the exclusion such a race would read as someone else's edit, and the
# hook would stay silent where it must speak.
stale_since() {
  local mark=$1 dir=$2 skip=${3:-} found
  [ -n "$mark" ] && [ -f "$mark" ] || return 1
  [ -d "$dir" ] || return 1

  found=$(
    find "$dir" \
      \( -name node_modules -o -name target -o -name dist -o -name .turbo \
         -o -name .git -o -name .next -o -name .build \) -prune -o \
      -type f -newer "$mark" ! -path "$skip" -print 2>/dev/null | head -1
  )
  [ -n "$found" ]
}

# stale_drop <marker> — removes the marker. Called from trap, silently.
stale_drop() {
  [ -n "${1:-}" ] && rm -f "$1" 2>/dev/null
  return 0
}
