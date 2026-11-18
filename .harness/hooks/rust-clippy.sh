#!/usr/bin/env bash
# Slot post-edit-async (PostToolUse/Write|Edit, asynchronous), module rust:
# lints the crate the file belongs to. Exit 2 wakes the model with the error
# text.
#
# Two fuses against a flood. A migration edit (changing a signature and moving
# dozens of calls onto it) touches files one by one, and until the last of them
# the crate is red for one and the same reason. Without the fuses every edit
# woke the session with the same text — ninety wake-ups in a row.
#
#   1. A lock per crate: while the previous run is going, a new one doesn't start.
#      It would look at the same worktree and say the same thing.
#   2. An error signature: only a new one wakes. The signature is the set of
#      `error` and `warning` lines without numbers, so shifted line numbers and
#      "due to N previous errors" don't change it, while a NEW error does and wakes.
#
# Neither can stay silent forever: clippy restarts on every edit, and the first
# change of the cause (or its disappearance) is heard.
#
# Third fuse: a stale verdict doesn't wake — see `lib/staleness.sh`.
#
# Test substitutes: `HARNESS_CLIPPY` — a command instead of `cargo clippy`,
# `HARNESS_CLIPPY_STATE` — a state directory instead of the one in the worktree's git dir.
set -uo pipefail

here=$(cd "${BASH_SOURCE[0]%/*}" && pwd)
# shellcheck source=lib/staleness.sh
. "$here/lib/staleness.sh"
# shellcheck source=lib/same-repo.sh
. "$here/lib/same-repo.sh"
# shellcheck source=lib/rust-crate.sh
. "$here/lib/rust-crate.sh"

input=$(cat)
# A subagent's edit doesn't wake the main session — see `js-typecheck.sh`. The
# executor is checked at the end of its work (`agent-checks.sh`).
[ -n "$(printf '%s' "$input" | jq -r '.agent_id // empty' 2>/dev/null)" ] && exit 0
f=$(printf '%s' "$input" | jq -r '.tool_response.filePath // .tool_input.file_path // empty' 2>/dev/null) || exit 0
case "$f" in
  *.rs) ;;
  *) exit 0 ;;
esac

# The file's crate and its directory — `lib/rust-crate.sh`.
crate_pkg=""
crate_dir=""
crate_file=""
rust_crate "$f" || exit 0
pkg=$crate_pkg
f=$crate_file

# State is per worktree: two worktrees lint different sources, and the
# signature of one must not suppress a wake-up in the other.
# The key is the worktree root, not the current directory: a subagent may make
# the edit from any subdirectory, and keyed by `$PWD` the state would split into
# several copies, and with it the whole point of the signature.
# The location is this worktree's git dir, not a shared temp one: in `/tmp` a
# directory with a predictable name can be created in advance by another user of
# the machine, and writing the signature would follow their link into a foreign file.
tree=${CLAUDE_PROJECT_DIR:-$(git rev-parse --show-toplevel 2>/dev/null || printf '%s' "$PWD")}
state=${HARNESS_CLIPPY_STATE:-$(git -C "$tree" rev-parse --path-format=absolute --git-path harness/clippy 2>/dev/null)}
[ -n "$state" ] || exit 0
mkdir -p "$state" 2>/dev/null || exit 0

lock="$state/$pkg.lock"
seen="$state/$pkg.seen"

# A lock older than ten minutes is the trace of a killed run, not a working one.
find "$lock" -maxdepth 0 -type d -mmin +10 -exec rmdir {} + 2>/dev/null

mkdir "$lock" 2>/dev/null || exit 0
mark=$(stale_marker)
trap 'rmdir "$lock" 2>/dev/null; stale_drop "$mark"' EXIT

out=$(rust_clippy "$pkg" "$crate_dir") && {
  # Green: the previous signature is forgotten, and the same error, if it returns, will wake.
  rm -f "$seen"
  exit 0
}

# The worktree moved on while we were checking: the batch of edits continues, and
# the verdict is no longer about what is on disk. Checked BEFORE the signature:
# recording the signature of a stale verdict would silence the next, fresh
# wake-up with the same cause.
if [ -n "$crate_dir" ] && stale_since "$mark" "$crate_dir" "$f"; then
  exit 0
fi

# Signature: error codes (E0061 and the like) plus text of errors without a code,
# without numbers. One and the same migration gives different wordings of one code
# ("takes 3 arguments but 2", "takes 2 arguments but 1") and different sets of
# files — by code they converge into one. "could not compile" lines are dropped:
# test names and counters jump around there, and there is no cause in them.
signature=$(printf '%s' "$out" | awk '
  /could not compile/ { next }
  /^error\[/ { code = $0; sub(/^error\[/, "", code); sub(/\].*/, "", code); print code; next }
  /^(error|warning)/ { gsub(/[0-9]+/, ""); print; next }
' | sort -u)
[ -n "$signature" ] || signature=$out
signature=$(printf '%s' "$signature" | cksum | cut -d' ' -f1)

[ "$(cat "$seen" 2>/dev/null)" = "$signature" ] && exit 0
printf '%s' "$signature" > "$seen"

printf 'clippy: %s fails -D warnings\n\n%s\n' "$pkg" "$(printf '%s' "$out" | tail -60)" >&2
exit 2
