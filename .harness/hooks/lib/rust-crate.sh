#!/usr/bin/env bash
# shellcheck disable=SC2034  # the output variables are read by the sourcing file
# The file's crate and the per-crate checks — shared by `rust-clippy.sh` (main
# session edits) and `agent-checks.sh` (end of the executor's work). Module rust.
#
# Requires `lib/same-repo.sh` to be sourced first.

# rust_crate <file> — the crate the file belongs to. Sets variables:
#   crate_pkg  — the package name for `cargo -p`;
#   crate_dir  — the crate directory: the area checked for verdict staleness,
#                and cargo's run directory (empty — the current one);
#   crate_file — the file path in the same form as `crate_dir`.
# Exit 1 — the file is not in a crate or not in our repository. Doesn't look
# at the extension: the caller decides which file deserves a check.
rust_crate() {
  local f=$1
  # A single crate at the root: the package is `rootstock`, the area is the
  # root of the project the file belongs to. The build directory is not source.
  local file_dir
  case "$f" in
    */target/* | target/*) return 1 ;;
  esac
  case "$f" in
    /*) file_dir=${f%/*} ;;
    */*) file_dir=$PWD/${f%/*} ;;
    *) file_dir=$PWD ;;
  esac
  # git in the file's directory — only when it is in a project worktree (see lib/same-repo.sh).
  same_repo "$file_dir" || return 1
  crate_dir=$(git -C "$file_dir" rev-parse --show-toplevel 2>/dev/null) || return 1
  [ -f "$crate_dir/Cargo.toml" ] || return 1
  # git gives the physical path (on macOS `/private/var/…` instead of `/var/…`); the
  # file path is brought to the same form, otherwise the staleness check wouldn't
  # recognise it.
  crate_file="$(cd "$file_dir" && pwd -P)/${f##*/}"
  crate_pkg="rootstock"
  # cargo executes build.rs, macros and the toolchain from rust-toolchain.toml: a
  # file in a foreign clone is no reason to run them.
  same_repo "${crate_dir:-.}"
}

# rust_clippy <package> <dir> — clippy for the crate with `-D warnings`, output
# to stdout, exit code is clippy's. The command can be substituted via `HARNESS_CLIPPY` (tests).
# Runs from the crate directory: a file of a neighbouring worktree is checked in its
# own worktree, not in the one the hook was called from.
rust_clippy() {
  local pkg=$1 dir=${2:-.}
  # Word splitting of `HARNESS_CLIPPY` is intentional: by default it is `cargo clippy`.
  # shellcheck disable=SC2086
  (cd "$dir" && ${HARNESS_CLIPPY:-cargo clippy} -p "$pkg" --all-targets --all-features -q -- -D warnings 2>&1)
}

# rust_fmt_check <file>… — rustfmt without writing: exit 1 and a list of differences
# if the files are formatted differently. Same edition as `rust-fmt.sh`.
# Colours are off: the output goes to the agent as text, and rustfmt colours even when not on a terminal.
rust_fmt_check() {
  rustfmt --edition 2024 --color never --check "$@" 2>&1
}
