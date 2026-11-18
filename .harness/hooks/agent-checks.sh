#!/usr/bin/env bash
# Slots subagent-stop (SubagentStop) and pre-handback (PreToolUse/SubagentHandback):
# the executor doesn't hand in its work while the fast checks for the packages it
# touched are red — rustfmt and clippy per crate (module rust), tsc per package
# (module frontend).
#
# Why. Subagent edits don't wake the asynchronous checks (`rust-clippy.sh`,
# `js-typecheck.sh`): the executor follows TDD, and red between its edits is a
# working move. But a report with red fmt, clippy or tsc was caught only by the
# main session's full run, and the executor had to be restarted. Here the check
# stands at the exit: on red the stop is held once, with the errors in the
# reason.
#
# An executor is an agent named `*-developer` (`rust-developer`,
# `frontend-developer` and project executors of the same kind): a naming
# convention of the template's agents; there is no second list that would fall
# behind the agents. Reviewers, built-in agents and the main session are of no
# interest to the hook.
#
# What is checked: files changed in the worktree relative to HEAD, and untracked
# ones (without ignored ones), reduced to crates and packages. Nothing changed —
# the hook is silent. The main session's changes before the executor started
# also get into the set: git doesn't know whose edit it is.
#
# Once, not in a loop:
#   - SubagentStop with `stop_hook_active` — the agent is already continuing
#     because of this same hook's denial; if it didn't fix it, let it return a
#     question in the report instead of spinning.
#   - SubagentHandback (Claude Code auto mode): the report goes out via this tool
#     BEFORE the stop, and it is the report that must be held, not the stop —
#     otherwise the red report is already with the main session. The denial is
#     once per agent; after that the report passes, and the same agent's stop
#     doesn't repeat the check. The marker is in the worktree's git dir
#     (`harness/agent-checks/<agent_id>`), like the `rust-clippy.sh` state.
#
# Test substitutes — the same as for the asynchronous hooks: `HARNESS_CLIPPY`,
# `HARNESS_TSC`.
set -uo pipefail

here=$(cd "${BASH_SOURCE[0]%/*}" && pwd)
# shellcheck source=lib/same-repo.sh
. "$here/lib/same-repo.sh"
have_rust=0
have_js=0
# Outputs of `rust_crate` and `js_package`: the libraries don't exist in every layout.
crate_pkg=""
crate_dir=""
pkg_dir=""
if [ -f "$here/lib/rust-crate.sh" ]; then
  # shellcheck source=/dev/null
  . "$here/lib/rust-crate.sh"
  have_rust=1
fi
if [ -f "$here/lib/js-package.sh" ]; then
  # shellcheck source=/dev/null
  . "$here/lib/js-package.sh"
  have_js=1
fi

# How many output lines of one check and of the whole reason go to the agent: it
# needs the first error and its location, not a wall of text in its context.
per_check_lines=30
reason_lines=90

input=$(cat)
field() { printf '%s' "$input" | jq -r "$1" 2>/dev/null; }

case "$(field '.agent_type // empty')" in
  *-developer) ;;
  *) exit 0 ;;
esac
event=$(field '.hook_event_name // empty')
case "$event" in
  # Stop — the executor as a separate session `claude --agent <executor>`: the
  # template doesn't wire it, a project may (README next door, "Executor checks").
  SubagentStop | Stop) [ "$(field '.stop_hook_active // false')" = true ] && exit 0 ;;
  PreToolUse) [ "$(field '.tool_name // empty')" = SubagentHandback ] || exit 0 ;;
  *) exit 0 ;;
esac

cwd=$(field '.cwd // empty')
[ -n "$cwd" ] || cwd=$PWD
top=$(git -C "$cwd" rev-parse --show-toplevel 2>/dev/null) || exit 0
# The checks execute the worktree's code (build.rs, node_modules) — only our own.
same_repo "$top" || exit 0
cd "$top" || exit 0

# The agent's report marker: `blocked` — the report was already held, `checked` —
# checked green. The name is agent_id without characters that don't belong in a file name.
id=$(field '.agent_id // empty' | LC_ALL=C tr -c 'A-Za-z0-9_.-' '_')
state=$(git rev-parse --path-format=absolute --git-path harness/agent-checks 2>/dev/null)
mark=""
[ -n "$id" ] && [ -n "$state" ] && mark="$state/$id"
# Denying the report without a marker would repeat on every attempt, and after the
# third one Claude Code ends the agent with no report at all. Better to let it pass.
[ "$event" = PreToolUse ] && [ -z "$mark" ] && exit 0

if [ -n "$mark" ] && [ -f "$mark" ]; then
  # This agent's report was already checked (or already held): the stop that
  # follows doesn't run the checks a second time, and the marker goes away — an
  # agent continued later is checked anew.
  [ "$event" = SubagentStop ] && rm -f "$mark"
  exit 0
fi

remember() {  # <blocked|checked> — only for the report: the stop has its own protection
  [ "$event" = PreToolUse ] && [ -n "$mark" ] || return 0
  mkdir -p "$state" 2>/dev/null && printf '%s\n' "$1" > "$mark"
}

changed=$({
  git diff --name-only HEAD -- 2>/dev/null
  git ls-files --others --exclude-standard 2>/dev/null
} | LC_ALL=C sort -u)
[ -n "$changed" ] || exit 0

# Crates and packages — one line each, "package<TAB>dir" and "dir", without repeats.
crates=""
packages=""
rs_files=()
while IFS= read -r p; do
  [ -n "$p" ] || continue
  f="$top/$p"
  case "$p" in
    *.rs | Cargo.toml | */Cargo.toml)
      [ "$have_rust" -eq 1 ] || continue
      rust_crate "$f" || continue
      crates="$crates$crate_pkg"$'\t'"$crate_dir"$'\n'
      case "$p" in *.rs) [ -f "$f" ] && rs_files+=("$f") ;; esac
      ;;
    *.ts | *.tsx | tsconfig.json | */tsconfig.json)
      [ "$have_js" -eq 1 ] || continue
      js_package "$f" || continue
      packages="$packages$pkg_dir"$'\n'
      ;;
  esac
done <<EOF
$changed
EOF

report=""
add_report() {  # <heading> <output>
  local body
  body=$(printf '%s\n' "$2" | head -n "$per_check_lines")
  [ "$(printf '%s\n' "$2" | wc -l)" -gt "$per_check_lines" ] && body="$body"$'\n'"…"
  report="$report$1"$'\n'"$body"$'\n\n'
}

if [ "${#rs_files[@]}" -gt 0 ]; then
  out=$(rust_fmt_check "${rs_files[@]}") || add_report "rustfmt --check: files are not formatted (cargo fmt)" "$out"
fi
while IFS=$'\t' read -r pkg dir; do
  [ -n "$pkg" ] || continue
  out=$(rust_clippy "$pkg" "$dir") || add_report "clippy: $pkg fails -D warnings" "$out"
done <<EOF
$(printf '%s' "$crates" | LC_ALL=C sort -u)
EOF
while IFS= read -r dir; do
  [ -n "$dir" ] || continue
  out=$(js_tsc "$dir") || add_report "tsc: ${dir#"$top"/} fails type checking" "$out"
done <<EOF
$(printf '%s' "$packages" | LC_ALL=C sort -u)
EOF

if [ -z "$report" ]; then
  remember checked
  exit 0
fi
remember blocked

reason=$(printf '%s\n\n%s' \
  "Work not handed in: the fast checks for the touched packages are red. Fix them within the task and check again. If the fix is a decision outside the task, return a question and list what is red in the report: this hook won't stop you a second time." \
  "$report" | head -n "$reason_lines")

if [ "$event" = PreToolUse ]; then
  jq -nc --arg r "$reason" \
    '{hookSpecificOutput:{hookEventName:"PreToolUse",permissionDecision:"deny",permissionDecisionReason:$r}}'
else
  jq -nc --arg r "$reason" '{decision:"block",reason:$r}'
fi
exit 0
