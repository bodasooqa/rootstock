#!/usr/bin/env bash
# Tests of the harness hooks and the dispatcher. Every hook is checked with at
# least two cases: firing and passing.
#
# Lesson: a hook that can quietly do nothing is worse than a missing one. The
# first version of the type check silently got an empty string and exited 0,
# i.e. pretended the check had passed. Caught only by a deliberately broken
# type.
#
# Everything runs on synthetic repositories in temporary directories, not on
# the project repository: the suite doesn't depend on its state. Module hook
# cases (rust-fmt, rust-clippy — module rust; js-fmt, js-typecheck — module
# frontend) run only if the hook exists in the project.
#
# Run: .harness/hooks/tests/run.sh
set -uo pipefail

# Under a git hook (lefthook's pre-push) the environment has GIT_DIR, GIT_INDEX_FILE
# and the like: any git command then looks into the hook's repository, not the
# directory it was run in. Agent hooks are never called like that, so the
# inheritance is removed — otherwise the tests check the wrong setting and "go red"
# exactly where everything is fine.
while IFS='=' read -r name _; do
  case "$name" in GIT_*) unset "$name" ;; esac
done < <(env)
# Hooks take the project root from here; the tests set up the setting themselves.
unset CLAUDE_PROJECT_DIR

here=$(cd "${BASH_SOURCE[0]%/*}" && pwd)
hooks=$(dirname "$here")
root=$(cd "$hooks/../.." && pwd)

pass=0
fail=0
# All temporary directories are inside one, removed on exit. The directory
# inside the project (for biome) is removed separately.
base=$(mktemp -d)
jsdir=""

cleanup() {
  rm -rf "$base"
  [ -n "$jsdir" ] && rm -rf "$jsdir"
}
trap cleanup EXIT

ok()   { pass=$((pass + 1)); printf '  ok   %s\n' "$1"; }
bad()  { fail=$((fail + 1)); printf '  FAIL %s\n     expected: %s\n     got: %s\n' "$1" "$2" "${3:-<empty>}"; }
has()  { [ -e "$hooks/$1" ]; }

# A temporary directory removed on exit: prints the path.
new_tmp() {
  mktemp -d "$base/tmp.XXXXXX"
}

# A temporary git repository with an empty commit on main: prints the path.
# Called as `repo=$(new_repo) || exit 1`.
#
# The separation check is not overcaution. Once such a suite ran from a git hook
# with an inherited GIT_DIR: `git init` turned the real repository into a bare
# one, `worktree add` created a branch in it, and `commit` landed on the current
# branch. A test able to write into the repository under test is more dangerous
# than an untested hook.
new_repo() {
  local d top want
  d=$(mktemp -d "$base/repo.XXXXXX")
  git -C "$d" init -q -b main
  top=$(git -C "$d" rev-parse --show-toplevel 2>/dev/null)
  want=$(cd "$d" && pwd -P)
  if [ "$top" != "$want" ]; then
    printf 'STOP: the temporary repository did not separate (%s instead of %s). The suite must not touch the working repository.\n' "${top:-<none>}" "$want" >&2
    return 1
  fi
  git -C "$d" config user.email t@t
  git -C "$d" config user.name t
  git -C "$d" commit -q --allow-empty -m init
  printf '%s\n' "$d"
}

# check <name> <script> <json> <deny|context|silent> [working dir]
check() {
  local name=$1 script=$2 json=$3 expect=$4 dir=${5:-$root} out
  out=$(cd "$dir" && printf '%s' "$json" | "$hooks/$script" 2>/dev/null)
  case "$expect" in
    deny)
      case "$out" in *'"permissionDecision":"deny"'*) ok "$name" ;; *) bad "$name" "denial" "$out" ;; esac ;;
    context)
      case "$out" in *'additionalContext'*) ok "$name" ;; *) bad "$name" "a hint to the model" "$out" ;; esac ;;
    silent)
      [ -z "$out" ] && ok "$name" || bad "$name" "silence" "$out" ;;
    *) bad "$name" "a known expectation" "$expect" ;;
  esac
}

bash_ev()   { jq -nc --arg c "$1" '{tool_name:"Bash",tool_input:{command:$c}}'; }
file_ev()   { jq -nc --arg f "$1" '{tool_name:"Edit",tool_input:{file_path:$f}}'; }
prompt_ev() { jq -nc --arg p "$1" '{hook_event_name:"UserPromptSubmit",prompt:$p}'; }

plain=$(new_tmp)

echo "guard-staging.sh"
staging_repo=$(new_repo) || exit 1
git -C "$staging_repo" worktree add -q "$staging_repo/wt" -b wt
check "git add -A in the shared checkout"     guard-staging.sh "$(bash_ev 'git add -A')"                     deny    "$staging_repo"
check "git add . in the shared checkout"      guard-staging.sh "$(bash_ev 'git add .')"                      deny    "$staging_repo"
check "git commit -am in the shared checkout" guard-staging.sh "$(bash_ev 'git commit -am "edit"')"          deny    "$staging_repo"
check "targeted git add"                      guard-staging.sh "$(bash_ev 'git add src/lib.rs')"             silent  "$staging_repo"
check "-a inside the message text"            guard-staging.sh "$(bash_ev 'git commit -m "add -a to docs"')" silent  "$staging_repo"
check "git add -A in own worktree"            guard-staging.sh "$(bash_ev 'git add -A')"                     silent  "$staging_repo/wt"
check "outside a repository"                  guard-staging.sh "$(bash_ev 'git add -A')"                     silent  "$plain"

# The heredoc parse errs only on the side of denial: the hook doesn't see what is cut out.
check "here-string is not a heredoc"          guard-staging.sh "$(bash_ev 'cat <<<x; git add -A')"           deny    "$staging_repo"
check "shift in \$((…)) is not a heredoc"     guard-staging.sh \
  "$(bash_ev "$(printf 'echo $((1<<2)) x\ngit add -A\nx')")"                                         deny    "$staging_repo"
check "heredoc without a terminator"          guard-staging.sh \
  "$(bash_ev "$(printf 'cat <<EOF >note.md\ntext\nEOFX\ngit add -A')")"                             deny    "$staging_repo"
check "terminator is the unquoted word"       guard-staging.sh \
  "$(bash_ev "$(printf 'cat <<E.O >note.md\nEO\ngit add -A\nE.O\ngit add note.md')")"               silent  "$staging_repo"
check "tabs are stripped only with <<-"       guard-staging.sh \
  "$(bash_ev "$(printf 'cat <<EOF >note.md\ntext\n\tEOF\ngit add -A')")"                            deny    "$staging_repo"
check "git add -A in a heredoc body"          guard-staging.sh \
  "$(bash_ev "$(printf "cat <<'EOF' >note.md\ngit add -A\nEOF\ngit add note.md")")"                 silent  "$staging_repo"
check "git add -A in a <<- body with a tab"   guard-staging.sh \
  "$(bash_ev "$(printf 'cat <<-"EOF" >note.md\n\tgit add -A\n\tEOF\ngit add note.md')")"            silent  "$staging_repo"
check "commit -F- with text in a heredoc"     guard-staging.sh \
  "$(bash_ev "$(printf "git commit -F- <<'EOF'\nfix: removed git add -A from the instructions\nEOF")")" silent  "$staging_repo"
# Doubt in the parse — the whole command is checked. In each case below real
# bash would either read the line as a command, or the hook isn't sure it wouldn't.
for c in \
  'cat <<\\EOF >note.md\ngit add -A\nEOF' \
  "cat <<\$'EOF' >note.md\ngit add -A\nEOF" \
  'cat <<$"EOF" >note.md\ngit add -A\nEOF' \
  'cat <<E$X >note.md\ngit add -A\nE$X' \
  'echo "<<EOF"\ngit add -A\nEOF' \
  "echo '<<EOF'\ngit add -A\nEOF" \
  '(( x <<EOF ))\ngit add -A\nEOF' \
  'echo $((1 <<EOF))\ngit add -A\nEOF' \
  'echo $[1 <<EOF]\ngit add -A\nEOF' \
  'let x=1<<EOF\ngit add -A\nEOF' \
  'echo x # <<EOF\ngit add -A\nEOF' \
  "echo \$'x'; cat <<EOF >note.md\ngit add -A\nEOF" \
  'echo $"x"; cat <<EOF >note.md\ngit add -A\nEOF' \
  'echo $(( 1 + "2" )); cat <<EOF >note.md\ngit add -A\nEOF' \
  'echo $[ "1" ]; cat <<EOF >note.md\ngit add -A\nEOF' \
  'let "x = 1"; cat <<EOF >note.md\ngit add -A\nEOF' \
  'cat <<EOF >note.md \\\ngit add -A\nEOF'; do
  check "not a heredoc: ${c%%\\n*}" guard-staging.sh "$(bash_ev "$(printf '%b' "$c")")" deny "$staging_repo"
done
check "a command after a heredoc is visible"  guard-staging.sh \
  "$(bash_ev "$(printf "cat <<'EOF' >note.md\ntext\nEOF\ngit add -A")")"                            deny    "$staging_repo"

echo
echo "bash-edit.sh"
check "edit via sed -i"               bash-edit.sh "$(bash_ev "sed -i '' 's/a/b/' src/lib.rs")"  context
check "redirect into a source file"   bash-edit.sh "$(bash_ev 'cat > src/app.tsx')"              context
check "build"                         bash-edit.sh "$(bash_ev 'git status')"                     silent
check "write into a temp directory"   bash-edit.sh "$(bash_ev 'echo done > /tmp/out.rs')"        silent
check "edit in a heredoc body"        bash-edit.sh \
  "$(bash_ev "$(printf 'cat <<%s\nsed -i x src/lib.rs\nEOF\n' "'EOF'")")"                         silent

echo
echo "session-tree.sh"
check "inside a repository"           session-tree.sh '{}'                                       context "$staging_repo"
check "outside a repository"          session-tree.sh '{}'                                       silent  "$plain"

# A session started in a foreign clone doesn't execute its code: `knowledge/due.sh`
# is taken from the hook's project, as in knowledge-due.sh, not from the directory's worktree.
foreign=$(new_repo) || exit 1
mkdir -p "$foreign/knowledge"
printf '#!/usr/bin/env bash\n: > "%s/foreign-due-ran"\n' "$base" > "$foreign/knowledge/due.sh"
chmod +x "$foreign/knowledge/due.sh"
(cd "$foreign" && printf '{}' | "$hooks/session-tree.sh" >/dev/null 2>&1)
if [ -e "$base/foreign-due-ran" ]; then
  bad "a foreign clone's due.sh is not executed" "script not run" "run"
else
  ok "a foreign clone's due.sh is not executed"
fi

out=$(cd "$staging_repo" && printf '{}' | "$hooks/session-tree.sh" 2>/dev/null)
case "$out" in
  *"SHARED checkout"*) ok "the shared checkout is named" ;;
  *) bad "the shared checkout is named" "a line about the shared checkout" "$out" ;;
esac
out=$(cd "$staging_repo/wt" && printf '{}' | "$hooks/session-tree.sh" 2>/dev/null)
case "$out" in
  *"in its own worktree"*"Other worktrees"*) ok "own worktree and neighbours are named" ;;
  *) bad "own worktree and neighbours are named" "own worktree and a list of neighbours" "$out" ;;
esac

# A build directory shared by all worktrees: reported exactly when it is configured.
out=$(cd "$staging_repo" && printf '{}' | "$hooks/session-tree.sh" 2>/dev/null)
case "$out" in
  *"A shared build directory is configured"*) bad "own build directory is fine" "silence about target-dir" "$out" ;;
  *) ok "own build directory is fine" ;;
esac
mkdir -p "$staging_repo/.cargo"
printf '[build]\ntarget-dir = "/tmp/shared"\n' > "$staging_repo/.cargo/config.toml"
out=$(cd "$staging_repo" && printf '{}' | "$hooks/session-tree.sh" 2>/dev/null)
case "$out" in
  *"A shared build directory is configured"*) ok "a leftover shared directory is named" ;;
  *) bad "a leftover shared directory is named" "a complaint about target-dir" "$out" ;;
esac
rm -rf "$staging_repo/.cargo"

# Git hooks installed from a worktree point into its node_modules and die together
# with it. A report — when the path is not in the main checkout or the file is
# missing; silence — when the path is right or the repository doesn't use lefthook.
main_phys=$(cd "$staging_repo" && pwd -P)
lefthook_hooks() {  # <binary path> — both hooks in the form lefthook 2 writes
  local h
  for h in pre-commit pre-push; do
    printf '#!/bin/sh\ncall_lefthook()\n{\n  if test -n "$LEFTHOOK_BIN"\n  then\n    "$LEFTHOOK_BIN" "$@"\n  elif lefthook -h >/dev/null 2>&1\n  then\n    lefthook "$@"\n  elif %s -h >/dev/null 2>&1\n  then\n    %s "$@"\n  fi\n}\n' "$1" "$1" > "$staging_repo/.git/hooks/$h"
  done
}
hooks_report() {  # <name> <broken|fine> [dir]
  out=$(cd "${3:-$staging_repo}" && printf '{}' | "$hooks/session-tree.sh" 2>/dev/null)
  case "$2:$out" in
    broken:*"Git hooks are broken"*"lefthook install"*) ok "$1" ;;
    broken:*) bad "$1" "a report about broken hooks" "$out" ;;
    fine:*"Git hooks are broken"*) bad "$1" "silence about git hooks" "$out" ;;
    fine:*) ok "$1" ;;
  esac
}
lefthook_hooks "$main_phys/wt/node_modules/lefthook/bin/lefthook"
hooks_report "without lefthook.yml silent about hooks" fine
printf 'pre-commit: {}\n' > "$staging_repo/lefthook.yml"
# The binary exists in the worktree too: the path there is wrong even while the worktree is alive.
for d in "$staging_repo" "$staging_repo/wt"; do
  mkdir -p "$d/node_modules/lefthook/bin"
  printf '#!/bin/sh\n' > "$d/node_modules/lefthook/bin/lefthook"
  chmod +x "$d/node_modules/lefthook/bin/lefthook"
done
lefthook_hooks "$main_phys/node_modules/lefthook/bin/lefthook"
hooks_report "hooks point into the main checkout — silent" fine
hooks_report "from a worktree, hooks in the main checkout — silent" fine "$staging_repo/wt"
lefthook_hooks "$main_phys/wt/node_modules/lefthook/bin/lefthook"
hooks_report "hooks point into a worktree — report" broken "$staging_repo/wt"
lefthook_hooks "$main_phys/node_modules/gone/bin/lefthook"
hooks_report "path into the main checkout, file missing — report" broken
# lefthook installed globally: a path outside node_modules must exist.
mkdir -p "$plain/global/bin"
printf '#!/bin/sh\n' > "$plain/global/bin/lefthook"
chmod +x "$plain/global/bin/lefthook"
lefthook_hooks "$plain/global/bin/lefthook"
hooks_report "global lefthook in place — silent" fine
lefthook_hooks "$plain/gone/bin/lefthook"
hooks_report "global lefthook gone — report" broken
rm -f "$staging_repo/.git/hooks/pre-commit" "$staging_repo/.git/hooks/pre-push"
hooks_report "no hooks — report" broken
rm -rf "$staging_repo/lefthook.yml" "$staging_repo/node_modules" "$staging_repo/wt/node_modules"

echo
echo "knowledge-due.sh"
# The hook fetches the report from its project's `knowledge/due.sh` (two levels up
# from the hooks directory). The project is synthetic: its own answers, its own
# stage table, a question for the stage in progress.
dueproj=$(new_tmp)
mkdir -p "$dueproj/.harness" "$dueproj/knowledge" "$dueproj/docs"
cp -R "$hooks" "$dueproj/.harness/hooks"
if [ -f "$root/knowledge/due.sh" ]; then
  cp "$root/knowledge/due.sh" "$dueproj/knowledge/"
  printf '[answers]\nopen_questions_path = "knowledge/open-questions.md"\nstages_path = "docs/stages.md"\n' > "$dueproj/.rootstock.toml"
  printf '| Stage | State |\n|---|---|\n| 1. Skeleton | in progress |\n' > "$dueproj/docs/stages.md"
  printf '# Open questions\n\n## Question\n\nGist.\n\nDue: stage 1.\n' > "$dueproj/knowledge/open-questions.md"
  due_check() {  # <name> <prompt> <context|silent>
    local out
    out=$(printf '%s' "$(prompt_ev "$2")" | "$dueproj/.harness/hooks/knowledge-due.sh" 2>/dev/null)
    case "$3:$out" in
      context:*additionalContext*"Open questions for stage 1"*) ok "$1" ;;
      context:*) bad "$1" "a reference on stage 1 questions" "$out" ;;
      silent:) ok "$1" ;;
      silent:*) bad "$1" "silence" "$out" ;;
    esac
  }
  due_check "before creating a change"  '/opsx:propose new screen'  context
  due_check "an ordinary message"       'fix the test'              silent
  due_check "apply of the same change"  '/opsx:apply stage1-screen' silent
else
  bad "knowledge/due.sh in place" "$root/knowledge/due.sh" "missing"
fi
# A reference that broke sending a message costs more than a missing reference:
# without knowledge/due.sh the hook is silent with exit 0.
rm -f "$dueproj/knowledge/due.sh"
out=$(printf '%s' "$(prompt_ev '/opsx:propose something')" | "$dueproj/.harness/hooks/knowledge-due.sh" 2>&1); code=$?
{ [ $code -eq 0 ] && [ -z "$out" ]; } && ok "without knowledge/due.sh silent" || bad "without knowledge/due.sh silent" "silence, exit 0" "exit $code: $out"

echo
echo "session-leftovers.sh"
tails=$(new_repo) || exit 1
out=$(cd "$tails" && "$hooks/session-leftovers.sh" 2>/dev/null)
[ -z "$out" ] && ok "clean worktree — silent" || bad "clean worktree — silent" "silence" "$out"

printf 'draft\n' > "$tails/draft.txt"
out=$(cd "$tails" && "$hooks/session-leftovers.sh" 2>/dev/null)
case "$out" in
  *"uncommitted paths: 1"*) ok "names uncommitted work" ;;
  *) bad "names uncommitted work" "a line about uncommitted paths" "$out" ;;
esac

# `node_modules` is symlinked into every worktree by the worktree setting and is
# not the session's work: the hook must not count it.
rm "$tails/draft.txt"
mkdir "$tails/node_modules"
printf '{}\n' > "$tails/node_modules/package.json"
out=$(cd "$tails" && "$hooks/session-leftovers.sh" 2>/dev/null)
[ -z "$out" ] && ok "node_modules is not a leftover" || bad "node_modules is not a leftover" "silence" "$out"

# Own branch with its own commit and no upstream — nobody has seen it yet.
git -C "$tails" checkout -q -b feature
git -C "$tails" commit -q --allow-empty -m "own work"
out=$(cd "$tails" && "$hooks/session-leftovers.sh" 2>/dev/null)
case "$out" in
  *"has never been pushed"*) ok "names an unpushed branch" ;;
  *) bad "names an unpushed branch" "a line about an unpushed branch" "$out" ;;
esac

out=$(cd "$plain" && "$hooks/session-leftovers.sh" 2>/dev/null)
[ -z "$out" ] && ok "outside a repository — silent" || bad "outside a repository — silent" "silence" "$out"

# Divergence with a neighbouring branch: two worktrees edit one file differently —
# two PRs on append-only journals.
git -C "$tails" checkout -q main
printf 'shared line\n' > "$tails/journal.md"
git -C "$tails" add journal.md
git -C "$tails" commit -q -m "journal"
git -C "$tails" worktree add -q "$tails/wt" -b neighbour
printf 'shared line\nneighbour entry\n' > "$tails/wt/journal.md"
git -C "$tails/wt" add journal.md
git -C "$tails/wt" commit -q -m "neighbour entry"

git -C "$tails" checkout -q -b mine
printf 'shared line\nmy entry\n' > "$tails/journal.md"
git -C "$tails" add journal.md
git -C "$tails" commit -q -m "my entry"

out=$(cd "$tails" && "$hooks/session-leftovers.sh" 2>/dev/null)
case "$out" in
  *"will diverge from branch neighbour"*journal.md*) ok "names divergence from a neighbouring branch" ;;
  *) bad "names divergence from a neighbouring branch" "a line about neighbour and journal.md" "$out" ;;
esac

# The same neighbour, but edits in different files — there is no divergence and
# it must not be invented: a report that always complains stops being read.
git -C "$tails" checkout -q -B mine2 main
printf 'mine\n' > "$tails/only-mine.md"
git -C "$tails" add only-mine.md
git -C "$tails" commit -q -m "only own file"
out=$(cd "$tails" && "$hooks/session-leftovers.sh" 2>/dev/null)
case "$out" in
  *"will diverge from branch"*) bad "non-overlapping edits are not divergence" "silence about neighbour" "$out" ;;
  *) ok "non-overlapping edits are not divergence" ;;
esac

fmtdir=$(new_tmp)
printf 'fn  main ( )  {\nlet x=1;\n}\n' > "$fmtdir/a.rs"
printf 'const  x  =1\n' > "$fmtdir/b.ts"

if has rust-fmt.sh; then
  echo
  echo "rust-fmt.sh"
  printf '%s' "$(file_ev "$fmtdir/a.rs")" | "$hooks/rust-fmt.sh" >/dev/null 2>&1
  grep -q '^fn main() {$' "$fmtdir/a.rs" && ok "formats .rs" || bad "formats .rs" "rustfmt applied" "$(cat "$fmtdir/a.rs")"
  before=$(cat "$fmtdir/b.ts")
  printf '%s' "$(file_ev "$fmtdir/b.ts")" | "$hooks/rust-fmt.sh" >/dev/null 2>&1
  [ "$before" = "$(cat "$fmtdir/b.ts")" ] && ok "leaves .ts alone" || bad "leaves .ts alone" "file unchanged" "$(cat "$fmtdir/b.ts")"
  printf 'fn  main ( )  {\nlet x=1;\n}\n' > "$fmtdir/a.rs"
fi

if has js-fmt.sh; then
  echo
  echo "js-fmt.sh"
  # Inside the project: biome comes from its node_modules, rules from its biome.json.
  jsdir="$root/.tmp-hook-test"
  mkdir -p "$jsdir"
  printf 'const  x  =   1;\nexport  {x}\n' > "$jsdir/a.ts"
  printf '%s' "$(file_ev "$jsdir/a.ts")" | "$hooks/js-fmt.sh" >/dev/null 2>&1
  grep -q '^const x = 1;$' "$jsdir/a.ts" && ok "formats .ts" || bad "formats .ts" "biome applied" "$(cat "$jsdir/a.ts")"
  before=$(cat "$fmtdir/a.rs")
  printf '%s' "$(file_ev "$fmtdir/a.rs")" | "$hooks/js-fmt.sh" >/dev/null 2>&1
  [ "$before" = "$(cat "$fmtdir/a.rs")" ] && ok "leaves .rs alone" || bad "leaves .rs alone" "file unchanged" "$(cat "$fmtdir/a.rs")"
  # A fixture is an external response byte for byte: biome would rewrite the number in it.
  mkdir -p "$jsdir/crates/client/tests/fixtures"
  printf '{"amount":  114187000000.000000000000000000}\n' > "$jsdir/crates/client/tests/fixtures/oi.json"
  fix_before=$(cat "$jsdir/crates/client/tests/fixtures/oi.json")
  printf '%s' "$(file_ev "$jsdir/crates/client/tests/fixtures/oi.json")" | "$hooks/js-fmt.sh" >/dev/null 2>&1
  [ "$fix_before" = "$(cat "$jsdir/crates/client/tests/fixtures/oi.json")" ] \
    && ok "leaves fixtures alone" || bad "leaves fixtures alone" "file unchanged" "$(cat "$jsdir/crates/client/tests/fixtures/oi.json")"
  # A file of a neighbouring worktree: biome.json exclusions come from its worktree's
  # root, not from where the hook was run. The binary is the project's only: the
  # edited file may live in a foreign clone, and its node_modules must not be executed.
  if nb=$(new_repo); then
    mkdir -p "$nb/gen" "$nb/node_modules/.bin"
    printf '{"files":{"includes":["**","!gen/**"]}}\n' > "$nb/biome.json"
    printf '#!/bin/sh\ntouch "%s/ran"\n' "$nb" > "$nb/node_modules/.bin/biome"
    chmod +x "$nb/node_modules/.bin/biome"
    printf 'const  x  =   1;\n' > "$nb/gen/a.ts"
    printf 'const  x  =   1;\n' > "$nb/b.ts"
    (cd "$root" && printf '%s' "$(file_ev "$nb/gen/a.ts")" | "$hooks/js-fmt.sh" >/dev/null 2>&1)
    grep -q '^const  x  =   1;$' "$nb/gen/a.ts" \
      && ok "neighbouring worktree: its biome.json exclusion" || bad "neighbouring worktree: its biome.json exclusion" "file unchanged" "$(cat "$nb/gen/a.ts")"
    (cd "$root" && printf '%s' "$(file_ev "$nb/b.ts")" | "$hooks/js-fmt.sh" >/dev/null 2>&1)
    grep -q '^const x = 1;$' "$nb/b.ts" \
      && ok "neighbouring worktree: formats" || bad "neighbouring worktree: formats" "biome applied" "$(cat "$nb/b.ts")"
    [ ! -e "$nb/ran" ] \
      && ok "neighbouring worktree: its biome is not executed" || bad "neighbouring worktree: its biome is not executed" "the project binary" "ran $nb/node_modules/.bin/biome"
  else
    bad "neighbouring worktree" "a temporary repository" "did not separate"
  fi
fi

if has rust-clippy.sh; then
  echo
  echo "rust-clippy.sh — path parsing and the error signature against a flood"
  out=$(printf '%s' "$(file_ev "$fmtdir/b.ts")" | "$hooks/rust-clippy.sh" 2>&1); code=$?
  { [ $code -eq 0 ] && [ -z "$out" ]; } && ok "clippy skips non-.rs" || bad "clippy skips non-.rs" "silence, exit 0" "exit $code: $out"
  out=$(printf '%s' "$(file_ev "$fmtdir/a.rs")" | "$hooks/rust-clippy.sh" 2>&1); code=$?
  { [ $code -eq 0 ] && [ -z "$out" ]; } && ok "clippy skips a file outside a crate" || bad "clippy skips a file outside a crate" "silence, exit 0" "exit $code: $out"

  # A migration edit touches dozens of files, and until the last of them the crate
  # is red for one reason. The hook must wake once, not ninety times — and must
  # wake again when the cause changes. Real cargo isn't needed here and won't do:
  # what is checked is the decision to wake, not the lint itself. The project fits
  # both layouts: a crate in crates/, and also a root with Cargo.toml.
  clippydir=$(new_repo) || exit 1
  clippystate=$(new_tmp)
  mkdir -p "$clippydir/crates/core/src" "$clippydir/bin"
  printf '[workspace]\n' > "$clippydir/Cargo.toml"
  printf 'fn main() {}\n' > "$clippydir/crates/core/src/lib.rs"
  fake="$clippydir/bin/clippy"
  printf '%s\n' '#!/usr/bin/env bash' 'cat "$FAKE_CLIPPY_OUT"; exit 1' > "$fake"
  chmod +x "$fake"
  printf 'error[E0061]: this method takes 3 arguments but 2 arguments were supplied\nerror: could not compile `demo-core` (test "a") due to 14 previous errors\n' \
    > "$clippystate/first.txt"
  # The same error code, but a different wording, a different file and a different
  # counter — exactly what a migration one call at a time gives. The signature must match.
  printf 'error[E0061]: this method takes 2 arguments but 1 argument was supplied\nerror: could not compile `demo-core` (test "b") due to 9 previous errors\n' \
    > "$clippystate/same.txt"
  printf 'error[E0599]: no method named `since_ms` found\nerror: could not compile `demo-core` (lib) due to 1 previous error\n' \
    > "$clippystate/other.txt"

  clippy_run() {
    CLAUDE_PROJECT_DIR="$clippydir" \
    FAKE_CLIPPY_OUT="$clippystate/$1.txt" \
    HARNESS_CLIPPY="$fake" \
    HARNESS_CLIPPY_STATE="$clippystate/state" \
      bash -c 'printf "%s" "$1" | "$2" 2>&1' _ \
        "$(file_ev "$clippydir/crates/core/src/lib.rs")" "$hooks/rust-clippy.sh"
  }

  out=$(clippy_run first); code=$?
  { [ $code -eq 2 ] && [ -n "$out" ]; } \
    && ok "the first error wakes" || bad "the first error wakes" "exit 2 with text" "exit $code: $out"
  out=$(clippy_run same); code=$?
  { [ $code -eq 0 ] && [ -z "$out" ]; } \
    && ok "the same error a second time is silent" || bad "the same error a second time is silent" "silence, exit 0" "exit $code: $out"
  out=$(clippy_run other); code=$?
  { [ $code -eq 2 ] && [ -n "$out" ]; } \
    && ok "a new error wakes" || bad "a new error wakes" "exit 2 with text" "exit $code: $out"
  # A green run forgets the signature: a returning error must wake again.
  printf '%s\n' '#!/usr/bin/env bash' 'exit 0' > "$fake"
  clippy_run other >/dev/null 2>&1
  printf '%s\n' '#!/usr/bin/env bash' 'cat "$FAKE_CLIPPY_OUT"; exit 1' > "$fake"
  out=$(clippy_run other); code=$?
  { [ $code -eq 2 ] && [ -n "$out" ]; } \
    && ok "after green the same error wakes" || bad "after green the same error wakes" "exit 2 with text" "exit $code: $out"

  # The executor follows TDD: red between its edits is a working move, not a
  # signal. The subagent marker is the event's `agent_id` field.
  agent_ev() { jq -nc --arg f "$1" '{tool_name:"Edit",tool_input:{file_path:$f},agent_id:"a1",agent_type:"developer"}'; }
  rm -rf "$clippystate/state"
  out=$(CLAUDE_PROJECT_DIR="$clippydir" FAKE_CLIPPY_OUT="$clippystate/first.txt" HARNESS_CLIPPY="$fake" HARNESS_CLIPPY_STATE="$clippystate/state" \
    bash -c 'printf "%s" "$1" | "$2" 2>&1' _ "$(agent_ev "$clippydir/crates/core/src/lib.rs")" "$hooks/rust-clippy.sh"); code=$?
  { [ $code -eq 0 ] && [ -z "$out" ]; } && ok "clippy is silent on a subagent edit" || bad "clippy is silent on a subagent edit" "silence, exit 0" "exit $code: $out"
  out=$(clippy_run first); code=$?
  { [ $code -eq 2 ] && [ -n "$out" ]; } && ok "clippy wakes on the same edit by the main session" || bad "clippy wakes on the same edit by the main session" "exit 2 with text" "exit $code: $out"

  # The state doesn't live in a shared temp directory: another user of the machine
  # would plant it there in advance, and the signature file would be a link to a
  # foreign file. The state's place is the project worktree's git dir.
  shared_tmp=$(new_tmp)
  rm -rf "$clippydir/.git/harness"
  out=$(CLAUDE_PROJECT_DIR="$clippydir" FAKE_CLIPPY_OUT="$clippystate/first.txt" HARNESS_CLIPPY="$fake" TMPDIR="$shared_tmp" \
    bash -c 'printf "%s" "$1" | "$2" 2>&1' _ "$(file_ev "$clippydir/crates/core/src/lib.rs")" "$hooks/rust-clippy.sh"); code=$?
  { [ $code -eq 2 ] && [ ! -e "$shared_tmp/harness-clippy-hook" ] && [ -d "$clippydir/.git/harness/clippy" ]; } \
    && ok "clippy state is in the project's git dir" \
    || bad "clippy state is in the project's git dir" "exit 2, state in .git/harness/clippy" "exit $code, in temp: $(ls "$shared_tmp")"

  # Edits come in a batch: while the check runs, the executor touches more files.
  # A verdict about a state no longer on disk must not wake. The fake linter
  # touches a neighbouring file itself while "checking" — so the race window is
  # reproduced exactly, not by timing.
  staledir=$(new_repo) || exit 1
  stalestate=$(new_tmp)
  mkdir -p "$staledir/crates/core/src" "$staledir/bin"
  printf '[workspace]\n' > "$staledir/Cargo.toml"
  printf 'fn main() {}\n' > "$staledir/crates/core/src/lib.rs"
  printf 'pub fn a() {}\n' > "$staledir/crates/core/src/other.rs"
  stalefake="$staledir/bin/clippy"
  printf '%s\n' '#!/usr/bin/env bash' \
    'sleep 0.05' \
    '[ -n "${FAKE_TOUCH:-}" ] && touch "$FAKE_TOUCH"' \
    'printf "error[E0061]: broken\n"' \
    'exit 1' > "$stalefake"
  chmod +x "$stalefake"
  sleep 1

  stale_run() {
    CLAUDE_PROJECT_DIR="$staledir" \
    FAKE_TOUCH=${1:-} \
    HARNESS_CLIPPY="$stalefake" \
    HARNESS_CLIPPY_STATE="$stalestate/state-$2" \
      bash -c 'printf "%s" "$1" | "$2" 2>&1' _ \
        "$(file_ev "$staledir/crates/core/src/lib.rs")" "$hooks/rust-clippy.sh"
  }

  out=$(stale_run "$staledir/crates/core/src/other.rs" a); code=$?
  { [ $code -eq 0 ] && [ -z "$out" ]; } \
    && ok "clippy is silent when the worktree moved on" \
    || bad "clippy is silent when the worktree moved on" "silence, exit 0" "exit $code: $out"
  out=$(stale_run "" b); code=$?
  { [ $code -eq 2 ] && [ -n "$out" ]; } \
    && ok "clippy wakes when the verdict is fresh" \
    || bad "clippy wakes when the verdict is fresh" "exit 2 with text" "exit $code: $out"
  # An edit of the very file isn't staleness: `rust-fmt.sh` rewrites exactly it
  # and could do so after the marker.
  out=$(stale_run "$staledir/crates/core/src/lib.rs" c); code=$?
  { [ $code -eq 2 ] && [ -n "$out" ]; } \
    && ok "clippy wakes if only the edited file itself was touched" \
    || bad "clippy wakes if only the edited file itself was touched" "exit 2 with text" "exit $code: $out"

  # A file of a foreign clone: cargo would execute its build.rs and macros there.
  rm -f "$staledir/ran"
  printf '%s\n' '#!/usr/bin/env bash' "touch '$staledir/ran'" 'exit 1' > "$stalefake"
  out=$(CLAUDE_PROJECT_DIR="$clippydir" HARNESS_CLIPPY="$stalefake" HARNESS_CLIPPY_STATE="$stalestate/state-d" \
    bash -c 'printf "%s" "$1" | "$2" 2>&1' _ "$(file_ev "$staledir/crates/core/src/lib.rs")" "$hooks/rust-clippy.sh"); code=$?
  { [ $code -eq 0 ] && [ -z "$out" ] && [ ! -e "$staledir/ran" ]; } \
    && ok "clippy doesn't run on a file of a foreign repository" \
    || bad "clippy doesn't run on a file of a foreign repository" "silence, exit 0, no run" "exit $code: $out$([ -e "$staledir/ran" ] && echo ' (ran)')"
  # A planted `.git` pointing at the project's gitdir doesn't make it ours: its
  # common git dir is the project's, but it isn't in the project's worktree list.
  forged=$(new_tmp)
  mkdir -p "$forged/crates/core/src"
  printf '[workspace]\n' > "$forged/Cargo.toml"
  printf 'fn main() {}\n' > "$forged/crates/core/src/lib.rs"
  printf 'gitdir: %s/.git\n' "$clippydir" > "$forged/.git"
  rm -f "$staledir/ran"
  out=$(CLAUDE_PROJECT_DIR="$clippydir" HARNESS_CLIPPY="$stalefake" HARNESS_CLIPPY_STATE="$stalestate/state-e" \
    bash -c 'printf "%s" "$1" | "$2" 2>&1' _ "$(file_ev "$forged/crates/core/src/lib.rs")" "$hooks/rust-clippy.sh"); code=$?
  { [ $code -eq 0 ] && [ -z "$out" ] && [ ! -e "$staledir/ran" ]; } \
    && ok "clippy doesn't run behind a planted .git" \
    || bad "clippy doesn't run behind a planted .git" "silence, exit 0, no run" "exit $code: $out$([ -e "$staledir/ran" ] && echo ' (ran)')"
  # A neighbouring worktree of the same repository is ours.
  git -C "$clippydir" add -A && git -C "$clippydir" commit -qm crate
  git -C "$clippydir" worktree add -q "$clippydir.wt" 2>/dev/null
  out=$(CLAUDE_PROJECT_DIR="$clippydir" FAKE_CLIPPY_OUT="$clippystate/first.txt" HARNESS_CLIPPY="$fake" HARNESS_CLIPPY_STATE="$clippystate/state-wt" \
    bash -c 'printf "%s" "$1" | "$2" 2>&1' _ "$(file_ev "$clippydir.wt/crates/core/src/lib.rs")" "$hooks/rust-clippy.sh"); code=$?
  { [ $code -eq 2 ] && [ -n "$out" ]; } \
    && ok "clippy runs on a neighbouring worktree of the same repository" \
    || bad "clippy runs on a neighbouring worktree of the same repository" "exit 2 with text" "exit $code: $out"

  # git's answer from a foreign directory can't be trusted: `core.worktree` in its
  # config names the project worktree as the root, while the run is in a foreign directory.
  # shellcheck disable=SC2329 # called via substitution
  clippy_foreign() {  # <file> <state name>
    rm -f "$staledir/ran"
    CLAUDE_PROJECT_DIR="$clippydir" HARNESS_CLIPPY="$stalefake" HARNESS_CLIPPY_STATE="$stalestate/state-$2" \
      bash -c 'printf "%s" "$1" | "$2" 2>&1' _ "$(file_ev "$1")" "$hooks/rust-clippy.sh"
  }
  if redirected=$(new_repo); then
    mkdir -p "$redirected/crates/core/src"
    printf '[workspace]\n' > "$redirected/Cargo.toml"
    printf 'fn main() {}\n' > "$redirected/crates/core/src/lib.rs"
    git -C "$redirected" config core.worktree "$clippydir"
    out=$(clippy_foreign "$redirected/crates/core/src/lib.rs" f); code=$?
    { [ $code -eq 0 ] && [ -z "$out" ] && [ ! -e "$staledir/ran" ]; } \
      && ok "clippy doesn't trust a foreign clone's core.worktree" \
      || bad "clippy doesn't trust a foreign clone's core.worktree" "silence, exit 0, no run" "exit $code: $out$([ -e "$staledir/ran" ] && echo ' (ran)')"
  fi
  # A clone inside the project worktree is foreign too: by git its root is itself.
  nested="$clippydir/vendor/other"
  mkdir -p "$nested/crates/core/src"
  git -C "$nested" init -q
  printf '[workspace]\n' > "$nested/Cargo.toml"
  printf 'fn main() {}\n' > "$nested/crates/core/src/lib.rs"
  out=$(clippy_foreign "$nested/crates/core/src/lib.rs" g); code=$?
  { [ $code -eq 0 ] && [ -z "$out" ] && [ ! -e "$staledir/ran" ]; } \
    && ok "clippy doesn't run in a nested clone" \
    || bad "clippy doesn't run in a nested clone" "silence, exit 0, no run" "exit $code: $out$([ -e "$staledir/ran" ] && echo ' (ran)')"
  # A nested clone whose core.worktree names the project worktree as the root.
  git -C "$nested" config core.worktree "$clippydir"
  out=$(clippy_foreign "$nested/crates/core/src/lib.rs" h); code=$?
  { [ $code -eq 0 ] && [ -z "$out" ] && [ ! -e "$staledir/ran" ]; } \
    && ok "clippy doesn't trust a nested clone's core.worktree" \
    || bad "clippy doesn't trust a nested clone's core.worktree" "silence, exit 0, no run" "exit $code: $out$([ -e "$staledir/ran" ] && echo ' (ran)')"
fi

if has js-typecheck.sh; then
  echo
  echo "js-typecheck.sh — path parsing, subagent, stale verdict"
  out=$(printf '%s' "$(file_ev "$fmtdir/a.rs")" | "$hooks/js-typecheck.sh" 2>&1); code=$?
  { [ $code -eq 0 ] && [ -z "$out" ]; } && ok "tsc skips non-.ts" || bad "tsc skips non-.ts" "silence, exit 0" "exit $code: $out"

  # The package fits both layouts: the packages/<name> directory, and also the root
  # of its own git repository with tsconfig.json. The compiler is the project's if
  # installed: otherwise a temp directory without node_modules would pull it from the network.
  tsc_bin=${HARNESS_TSC:-}
  [ -z "$tsc_bin" ] && [ -x "$root/node_modules/.bin/tsc" ] && tsc_bin="$root/node_modules/.bin/tsc"
  tsdir=$(new_tmp)
  for p in broken clean; do
    mkdir -p "$tsdir/packages/$p/src"
    git -C "$tsdir/packages/$p" init -q
    printf '{"compilerOptions":{"strict":true,"noEmit":true,"skipLibCheck":true,"types":[]},"include":["src"]}\n' > "$tsdir/packages/$p/tsconfig.json"
  done
  printf 'export const n: number = "string";\n' > "$tsdir/packages/broken/src/index.ts"
  printf 'export const n: number = 1;\n' > "$tsdir/packages/clean/src/index.ts"
  # Each package is its own repository, and the project for the hook is the file's package.
  ts_run() {  # <file> [event]
    CLAUDE_PROJECT_DIR="${1%/src/*}" HARNESS_TSC="$tsc_bin" bash -c 'printf "%s" "$1" | "$2" 2>&1' _ "${2:-$(file_ev "$1")}" "$hooks/js-typecheck.sh"
  }
  if [ -n "$tsc_bin" ]; then
    out=$(ts_run "$tsdir/packages/broken/src/index.ts"); code=$?
    { [ $code -eq 2 ] && [ -n "$out" ]; } && ok "tsc catches a broken type" || bad "tsc catches a broken type" "exit 2 with the error text" "exit $code: $out"
    out=$(ts_run "$tsdir/packages/clean/src/index.ts"); code=$?
    { [ $code -eq 0 ] && [ -z "$out" ]; } && ok "tsc is silent on a healthy package" || bad "tsc is silent on a healthy package" "silence, exit 0" "exit $code: $out"
  else
    bad "the TypeScript compiler in place" "$root/node_modules/.bin/tsc" "missing — bun install needed"
  fi
  out=$(ts_run "$tsdir/packages/broken/src/index.ts" \
    "$(jq -nc --arg f "$tsdir/packages/broken/src/index.ts" '{tool_name:"Edit",tool_input:{file_path:$f},agent_id:"a1"}')"); code=$?
  { [ $code -eq 0 ] && [ -z "$out" ]; } && ok "tsc is silent on a subagent edit" || bad "tsc is silent on a subagent edit" "silence, exit 0" "exit $code: $out"

  # The stand-in command touches a neighbouring file itself, and no race remains in the test.
  stalets="$tsdir/packages/broken"
  printf 'export const m = 1;\n' > "$stalets/src/other.ts"
  stalefakets="$tsdir/tsc-fake"
  printf '%s\n' '#!/usr/bin/env bash' \
    'sleep 0.05' \
    '[ -n "${FAKE_TOUCH:-}" ] && touch "$FAKE_TOUCH"' \
    'printf "src/index.ts(1,14): error TS2322: broken\n"' \
    'exit 1' > "$stalefakets"
  chmod +x "$stalefakets"
  sleep 1
  stale_ts_run() {
    CLAUDE_PROJECT_DIR="$stalets" FAKE_TOUCH=${1:-} HARNESS_TSC="$stalefakets" \
      bash -c 'printf "%s" "$1" | "$2" 2>&1' _ \
        "$(file_ev "$stalets/src/index.ts")" "$hooks/js-typecheck.sh"
  }
  out=$(stale_ts_run "$stalets/src/other.ts"); code=$?
  { [ $code -eq 0 ] && [ -z "$out" ]; } \
    && ok "tsc is silent when the worktree moved on" \
    || bad "tsc is silent when the worktree moved on" "silence, exit 0" "exit $code: $out"
  out=$(stale_ts_run ""); code=$?
  { [ $code -eq 2 ] && [ -n "$out" ]; } \
    && ok "tsc wakes when the verdict is fresh" \
    || bad "tsc wakes when the verdict is fresh" "exit 2 with text" "exit $code: $out"
  out=$(stale_ts_run "$stalets/src/index.ts"); code=$?
  { [ $code -eq 2 ] && [ -n "$out" ]; } \
    && ok "tsc wakes if only the edited file itself was touched" \
    || bad "tsc wakes if only the edited file itself was touched" "exit 2 with text" "exit $code: $out"

  # A file of a foreign clone: tsc from its node_modules is not executed.
  rm -f "$tsdir/ran"
  printf '%s\n' '#!/usr/bin/env bash' "touch '$tsdir/ran'" 'exit 1' > "$stalefakets"
  out=$(CLAUDE_PROJECT_DIR="$tsdir/packages/clean" HARNESS_TSC="$stalefakets" \
    bash -c 'printf "%s" "$1" | "$2" 2>&1' _ "$(file_ev "$stalets/src/index.ts")" "$hooks/js-typecheck.sh"); code=$?
  { [ $code -eq 0 ] && [ -z "$out" ] && [ ! -e "$tsdir/ran" ]; } \
    && ok "tsc doesn't run on a file of a foreign repository" \
    || bad "tsc doesn't run on a file of a foreign repository" "silence, exit 0, no run" "exit $code: $out$([ -e "$tsdir/ran" ] && echo ' (ran)')"

  # Something ignored in the project worktree is not the project: a package unpacked
  # into tmp/ would bring its own tsc. The package is at the root too — fits both layouts.
  if ign=$(new_repo); then
    printf 'tmp/\n' > "$ign/.gitignore"
    pkgdir="$ign/tmp/pkg/packages/x"
    mkdir -p "$pkgdir/src"
    for d in "$ign" "$pkgdir"; do
      printf '{"include":["src"]}\n' > "$d/tsconfig.json"
    done
    printf 'export const n = 1;\n' > "$pkgdir/src/index.ts"
    rm -f "$tsdir/ran"
    out=$(CLAUDE_PROJECT_DIR="$ign" HARNESS_TSC="$stalefakets" \
      bash -c 'printf "%s" "$1" | "$2" 2>&1' _ "$(file_ev "$pkgdir/src/index.ts")" "$hooks/js-typecheck.sh"); code=$?
    { [ $code -eq 0 ] && [ -z "$out" ] && [ ! -e "$tsdir/ran" ]; } \
      && ok "tsc doesn't run in an ignored directory" \
      || bad "tsc doesn't run in an ignored directory" "silence, exit 0, no run" "exit $code: $out$([ -e "$tsdir/ran" ] && echo ' (ran)')"
  fi
fi

echo
echo "agent-checks.sh — the executor doesn't finish with red checks"
# The project fits all layouts: Cargo.toml and tsconfig.json at the root (a single
# crate, a single app), a crate in crates/ and a package in packages/ (monorepo).
# Real clippy and tsc aren't needed: what is checked is what the hook runs, when,
# and what it answers. rustfmt is real: it is fast and present wherever module rust is.
ac=$(new_repo) || exit 1
acbin=$(new_tmp)
mkdir -p "$ac/crates/core/src" "$ac/packages/web/src"
printf '[workspace]\n' > "$ac/Cargo.toml"
printf 'pub fn a() {}\n' > "$ac/crates/core/src/lib.rs"
printf '{}\n' > "$ac/tsconfig.json"
printf '{}\n' > "$ac/packages/web/tsconfig.json"
printf 'export const n = 1;\n' > "$ac/packages/web/src/index.ts"
git -C "$ac" add -A && git -C "$ac" commit -qm base
# The stand-in checks log the call; they are red by an environment variable.
# `many` — long output: the denial reason must be compressed.
printf '%s\n' '#!/usr/bin/env bash' 'printf "clippy %s\n" "$*" >> "$FAKE_LOG"' \
  'case "${FAKE_CLIPPY_RED:-}" in' \
  '  "") exit 0 ;;' \
  '  many) for i in $(seq 1 500); do printf "error[E0308]: mismatched types %s\n" "$i"; done; exit 1 ;;' \
  '  *) printf "error[E0308]: mismatched types\n"; exit 1 ;;' \
  'esac' > "$acbin/clippy"
printf '%s\n' '#!/usr/bin/env bash' 'printf "tsc %s\n" "$PWD" >> "$FAKE_LOG"' \
  '[ -n "${FAKE_TSC_RED:-}" ] || exit 0' \
  'printf "src/index.ts(1,14): error TS2322: broken\n"; exit 1' > "$acbin/tsc"
chmod +x "$acbin/clippy" "$acbin/tsc"

ac_run() {  # <event> — the hook's stdout; the FAKE_* environment comes from the caller
  (cd "$ac" && printf '%s' "$1" | CLAUDE_PROJECT_DIR="$ac" HARNESS_CLIPPY="$acbin/clippy" \
    HARNESS_TSC="$acbin/tsc" FAKE_LOG="$acbin/log" "$hooks/agent-checks.sh" 2>/dev/null)
}
stop_ev() {  # <agent type> [stop_hook_active] [agent_id]
  jq -nc --arg t "$1" --argjson a "${2:-false}" --arg id "${3:-ag1}" --arg c "$ac" \
    '{hook_event_name:"SubagentStop",agent_id:$id,agent_type:$t,stop_hook_active:$a,cwd:$c}'
}
handback_ev() {  # <agent_id>
  jq -nc --arg id "$1" --arg c "$ac" \
    '{hook_event_name:"PreToolUse",tool_name:"SubagentHandback",tool_input:{message:"report"},agent_id:$id,agent_type:"rust-developer",cwd:$c}'
}
ac_ran() { [ -s "$acbin/log" ]; }

rm -f "$acbin/log"
out=$(FAKE_CLIPPY_RED=1 FAKE_TSC_RED=1 ac_run "$(stop_ev rust-developer)")
{ [ -z "$out" ] && ! ac_ran; } \
  && ok "executor without changes — silent and doesn't check" \
  || bad "executor without changes — silent and doesn't check" "silence, no run" "$out $(cat "$acbin/log" 2>/dev/null)"

# Worktree changes: an edit of a tracked file and a new file.
printf 'pub fn a() {}\npub fn b() {}\n' > "$ac/crates/core/src/lib.rs"
printf 'export const n = 2;\n' > "$ac/packages/web/src/index.ts"
printf 'export const m = 1;\n' > "$ac/packages/web/src/new.ts"

rm -f "$acbin/log"
out=$(FAKE_CLIPPY_RED=1 FAKE_TSC_RED=1 ac_run "$(stop_ev rust-reviewer)")
{ [ -z "$out" ] && ! ac_ran; } \
  && ok "not an executor — silent" || bad "not an executor — silent" "silence, no run" "$out"
out=$(FAKE_CLIPPY_RED=1 FAKE_TSC_RED=1 ac_run "$(jq -nc --arg c "$ac" '{hook_event_name:"SubagentStop",agent_id:"x",agent_type:"",stop_hook_active:false,cwd:$c}')")
{ [ -z "$out" ] && ! ac_ran; } \
  && ok "built-in agent without a type — silent" || bad "built-in agent without a type — silent" "silence, no run" "$out"
out=$(FAKE_CLIPPY_RED=1 FAKE_TSC_RED=1 ac_run "$(stop_ev rust-developer true)")
{ [ -z "$out" ] && ! ac_ran; } \
  && ok "stop_hook_active — not held a second time" || bad "stop_hook_active — not held a second time" "silence, no run" "$out"
out=$(ac_run "$(stop_ev frontend-developer)")
{ [ -z "$out" ] && ac_ran; } \
  && ok "executor, checks green — silent" || bad "executor, checks green — silent" "silence after a run" "$out / log: $(cat "$acbin/log" 2>/dev/null)"

if has lib/rust-crate.sh; then
  rm -f "$acbin/log"
  out=$(FAKE_CLIPPY_RED=1 ac_run "$(stop_ev rust-developer)")
  { [ "$(printf '%s' "$out" | jq -r '.decision' 2>/dev/null)" = block ] \
    && printf '%s' "$out" | jq -r '.reason' | grep -q 'E0308'; } \
    && ok "red clippy — block with output" || bad "red clippy — block with output" "decision block, E0308 in the reason" "$out"
  [ "$(grep -c '^clippy -p ' "$acbin/log")" -eq 1 ] \
    && ok "clippy once for the touched crate" \
    || bad "clippy once for the touched crate" "one clippy -p …" "$(cat "$acbin/log" 2>/dev/null)"
  out=$(FAKE_CLIPPY_RED=many ac_run "$(stop_ev rust-developer)")
  lines=$(printf '%s' "$out" | jq -r '.reason' 2>/dev/null | wc -l | tr -d ' ')
  { [ "$(printf '%s' "$out" | jq -r '.decision' 2>/dev/null)" = block ] && [ "$lines" -le 100 ]; } \
    && ok "long output compressed in the reason" || bad "long output compressed in the reason" "no more than 100 lines" "$lines lines"
  # A badly formatted new file: rustfmt --check goes red without clippy.
  printf 'fn  x ( )  {}\n' > "$ac/crates/core/src/bad.rs"
  out=$(ac_run "$(stop_ev rust-developer)")
  { [ "$(printf '%s' "$out" | jq -r '.decision' 2>/dev/null)" = block ] \
    && printf '%s' "$out" | jq -r '.reason' | grep -q 'rustfmt'; } \
    && ok "red rustfmt — block" || bad "red rustfmt — block" "decision block, rustfmt in the reason" "$out"
  rm -f "$ac/crates/core/src/bad.rs"
fi

if has lib/js-package.sh; then
  rm -f "$acbin/log"
  out=$(FAKE_TSC_RED=1 ac_run "$(stop_ev frontend-developer)")
  { [ "$(printf '%s' "$out" | jq -r '.decision' 2>/dev/null)" = block ] \
    && printf '%s' "$out" | jq -r '.reason' | grep -q 'TS2322'; } \
    && ok "red tsc — block with output" || bad "red tsc — block with output" "decision block, TS2322 in the reason" "$out"
  [ "$(grep -c '^tsc' "$acbin/log")" -eq 1 ] \
    && ok "tsc once per package, even with two files" || bad "tsc once per package, even with two files" "one run" "$(cat "$acbin/log")"
fi

# An executor in auto mode hands in its report with the SubagentHandback tool BEFORE
# the stop: that is what must be held, otherwise the red report is already gone. The
# denial is once, then the report passes, and the same agent's stop doesn't repeat the checks.
out=$(FAKE_CLIPPY_RED=1 FAKE_TSC_RED=1 ac_run "$(handback_ev h1)")
[ "$(printf '%s' "$out" | jq -r '.hookSpecificOutput.permissionDecision' 2>/dev/null)" = deny ] \
  && ok "SubagentHandback with red — denied" || bad "SubagentHandback with red — denied" "permissionDecision deny" "$out"
rm -f "$acbin/log"
out=$(FAKE_CLIPPY_RED=1 FAKE_TSC_RED=1 ac_run "$(handback_ev h1)")
{ [ -z "$out" ] && ! ac_ran; } \
  && ok "the second SubagentHandback passes" || bad "the second SubagentHandback passes" "silence, no run" "$out"
out=$(FAKE_CLIPPY_RED=1 FAKE_TSC_RED=1 ac_run "$(stop_ev rust-developer false h1)")
{ [ -z "$out" ] && ! ac_ran; } \
  && ok "stop after a denied report — no repeat" || bad "stop after a denied report — no repeat" "silence, no run" "$out"
out=$(ac_run "$(handback_ev h2)")
rm -f "$acbin/log"
out=$(FAKE_CLIPPY_RED=1 FAKE_TSC_RED=1 ac_run "$(stop_ev rust-developer false h2)")
{ [ -z "$out" ] && ! ac_ran; } \
  && ok "stop after a green report — no repeat" || bad "stop after a green report — no repeat" "silence, no run" "$out"

# An executor as a separate session `claude --agent`: Stop without agent_id, the agent type in agent_type.
out=$(FAKE_CLIPPY_RED=1 FAKE_TSC_RED=1 ac_run "$(jq -nc --arg c "$ac" '{hook_event_name:"Stop",agent_type:"rust-developer",stop_hook_active:false,cwd:$c}')")
[ "$(printf '%s' "$out" | jq -r '.decision' 2>/dev/null)" = block ] \
  && ok "Stop of an executor session — block" || bad "Stop of an executor session — block" "decision block" "$out"
out=$(FAKE_CLIPPY_RED=1 FAKE_TSC_RED=1 ac_run "$(jq -nc --arg c "$ac" '{hook_event_name:"Stop",stop_hook_active:false,cwd:$c}')")
[ -z "$out" ] && ok "Stop of the main session — silent" || bad "Stop of the main session — silent" "silence" "$out"

# Real wiring: the dispatcher passes the slot's denial out as is.
out=$(cd "$ac" && printf '%s' "$(stop_ev rust-developer false ag9)" | CLAUDE_PROJECT_DIR="$ac" \
  HARNESS_CLIPPY="$acbin/clippy" HARNESS_TSC="$acbin/tsc" FAKE_LOG="$acbin/log" FAKE_CLIPPY_RED=1 FAKE_TSC_RED=1 \
  "$hooks/dispatch.sh" subagent-stop 2>/dev/null)
[ "$(printf '%s' "$out" | jq -r '.decision' 2>/dev/null)" = block ] \
  && ok "dispatcher subagent-stop gives block" || bad "dispatcher subagent-stop gives block" "decision block" "$out"
out=$(cd "$ac" && printf '%s' "$(handback_ev h9)" | CLAUDE_PROJECT_DIR="$ac" \
  HARNESS_CLIPPY="$acbin/clippy" HARNESS_TSC="$acbin/tsc" FAKE_LOG="$acbin/log" FAKE_CLIPPY_RED=1 FAKE_TSC_RED=1 \
  "$hooks/dispatch.sh" pre-handback 2>/dev/null)
[ "$(printf '%s' "$out" | jq -r '.hookSpecificOutput.permissionDecision' 2>/dev/null)" = deny ] \
  && ok "dispatcher pre-handback gives a denial" || bad "dispatcher pre-handback gives a denial" "permissionDecision deny" "$out"

# Outside a repository — silence.
out=$(cd "$plain" && jq -nc --arg c "$plain" '{hook_event_name:"SubagentStop",agent_id:"a",agent_type:"rust-developer",stop_hook_active:false,cwd:$c}' \
  | "$hooks/agent-checks.sh" 2>/dev/null)
[ -z "$out" ] && ok "outside a repository — silent" || bad "outside a repository — silent" "silence" "$out"

echo
echo "dispatch.sh"
# A synthetic harness: the real dispatcher, fake common hooks of the slot,
# the project's own hooks. Each hook writes its name to the log — so the order is visible.
dproj=$(new_tmp)
mkdir -p "$dproj/.harness/hooks" "$dproj/.harness/local/hooks"
cp "$hooks/dispatch.sh" "$dproj/.harness/hooks/"
dlog="$dproj/log"
fake_hook() {  # <path> <name for the log> <body> — the body gets the event on stdin
  mkdir -p "$(dirname "$1")"
  printf '#!/usr/bin/env bash\nprintf "%%s\\n" %s >> %s\n%s\n' "$2" "$dlog" "$3" > "$1"
  chmod +x "$1"
}
dispatch() {  # <slot> [event] — sets d_code, d_out, d_err and d_log
  local out err code
  : > "$dlog"
  out=$(printf '%s' "${2:-{\}}" | "$dproj/.harness/hooks/dispatch.sh" "$1" 2>"$dproj/err")
  code=$?
  err=$(cat "$dproj/err")
  d_code=$code d_out=$out d_err=$err
  d_log=$(tr '\n' ' ' < "$dlog")
}
local_dir="$dproj/.harness/local/hooks"

# Order: the slot's common hooks in their order, then own ones alphabetically.
fake_hook "$dproj/.harness/hooks/rust-fmt.sh" rust-fmt ':'
fake_hook "$dproj/.harness/hooks/js-fmt.sh" js-fmt ':'
fake_hook "$local_dir/post-edit/20-b.sh" b ':'
fake_hook "$local_dir/post-edit/10-a.sh" a ':'
dispatch post-edit
[ "$d_log" = "rust-fmt js-fmt a b " ] && [ "$d_code" -eq 0 ] \
  && ok "order: common, then own alphabetically" || bad "order: common, then own alphabetically" "rust-fmt js-fmt a b" "exit $d_code: $d_log"

# A hook gets the whole event.
fake_hook "$local_dir/post-edit/10-a.sh" a 'jq -r .tool_input.file_path >> '"$dproj/seen"
dispatch post-edit "$(file_ev /x/y.rs)"
[ "$(cat "$dproj/seen" 2>/dev/null)" = "/x/y.rs" ] \
  && ok "a hook gets the event on stdin" || bad "a hook gets the event on stdin" "/x/y.rs" "$(cat "$dproj/seen" 2>/dev/null)"
fake_hook "$local_dir/post-edit/10-a.sh" a ':'

# A missing module hook is skipped.
rm "$dproj/.harness/hooks/js-fmt.sh"
dispatch post-edit
[ "$d_log" = "rust-fmt a b " ] \
  && ok "no module hook — skipped" || bad "no module hook — skipped" "rust-fmt a b" "$d_log"

# Exit 2 stops the slot and passes stderr through.
fake_hook "$local_dir/post-edit/10-a.sh" a 'echo "crate red" >&2; exit 2'
dispatch post-edit
{ [ "$d_code" -eq 2 ] && [ "$d_err" = "crate red" ] && [ "$d_log" = "rust-fmt a " ]; } \
  && ok "exit 2 stops and passes stderr through" \
  || bad "exit 2 stops and passes stderr through" "exit 2, \"crate red\", b not run" "exit $d_code, stderr \"$d_err\", log $d_log"
fake_hook "$dproj/.harness/hooks/rust-fmt.sh" rust-fmt 'echo "common red" >&2; exit 2'
dispatch post-edit
{ [ "$d_code" -eq 2 ] && [ "$d_log" = "rust-fmt " ]; } \
  && ok "exit 2 of a common hook keeps own ones from running" || bad "exit 2 of a common hook keeps own ones from running" "exit 2, only rust-fmt" "exit $d_code: $d_log"
fake_hook "$dproj/.harness/hooks/rust-fmt.sh" rust-fmt ':'
fake_hook "$local_dir/post-edit/10-a.sh" a ':'

# Empty local: only common ones.
fake_hook "$dproj/.harness/hooks/bash-edit.sh" bash-edit 'echo "text"'
dispatch post-bash
{ [ "$d_code" -eq 0 ] && [ "$d_log" = "bash-edit " ] && [ "$d_out" = "text" ]; } \
  && ok "without own hooks — only common ones" || bad "without own hooks — only common ones" "exit 0, bash-edit, \"text\"" "exit $d_code: $d_log / $d_out"
mkdir -p "$local_dir/post-bash"
dispatch post-bash
{ [ "$d_code" -eq 0 ] && [ "$d_log" = "bash-edit " ]; } \
  && ok "empty own hooks directory — only common ones" || bad "empty own hooks directory — only common ones" "bash-edit" "$d_log"

# An own hook without the execute bit is skipped with a warning.
fake_hook "$local_dir/post-bash/off.sh" off ':'
chmod -x "$local_dir/post-bash/off.sh"
dispatch post-bash
{ [ "$d_code" -eq 0 ] && [ "$d_log" = "bash-edit " ] && [[ "$d_err" == *"off.sh is not executable"* ]]; } \
  && ok "a non-executable own hook is skipped with a warning" \
  || bad "a non-executable own hook is skipped with a warning" "only bash-edit, a warning" "exit $d_code, log $d_log, stderr $d_err"
rm -f "$local_dir/post-bash/off.sh"

# A common denial and an own denial: one valid JSON goes out — the first denial.
deny_body() { printf 'jq -nc --arg r %s '\''{hookSpecificOutput:{hookEventName:"PreToolUse",permissionDecision:"deny",permissionDecisionReason:$r}}'\''' "$1"; }
fake_hook "$dproj/.harness/hooks/guard-staging.sh" guard-staging "$(deny_body 'common-denial')"
fake_hook "$local_dir/pre-bash/guard-data.sh" guard-data "$(deny_body 'own-denial')"
dispatch pre-bash "$(bash_ev 'git add -A')"
reason=$(printf '%s' "$d_out" | jq -r '.hookSpecificOutput.permissionDecisionReason' 2>/dev/null)
{ [ "$d_code" -eq 0 ] && [ "$reason" = "common-denial" ] && [ "$d_log" = "guard-staging " ]; } \
  && ok "two denials — one JSON, the first denial, slot stopped" \
  || bad "two denials — one JSON, the first denial, slot stopped" "JSON with \"common-denial\", guard-data not run" "exit $d_code, log $d_log, output $d_out"
fake_hook "$dproj/.harness/hooks/guard-staging.sh" guard-staging ':'
dispatch pre-bash "$(bash_ev 'git add -A')"
reason=$(printf '%s' "$d_out" | jq -r '.hookSpecificOutput.permissionDecisionReason' 2>/dev/null)
{ [ "$reason" = "own-denial" ] && [ "$d_log" = "guard-staging guard-data " ]; } \
  && ok "an own hook's denial gets through" || bad "an own hook's denial gets through" "JSON with \"own-denial\"" "$d_out"

# Two additionalContext — one JSON with the texts joined by a blank line.
ctx_body() { printf 'jq -nc --arg n %s '\''{hookSpecificOutput:{hookEventName:"SessionStart",additionalContext:$n}}'\''' "$1"; }
fake_hook "$dproj/.harness/hooks/session-tree.sh" session-tree "$(ctx_body 'first')"
fake_hook "$local_dir/session-start/note.sh" note "$(ctx_body 'second')"
dispatch session-start
ctx=$(printf '%s' "$d_out" | jq -r '.hookSpecificOutput.additionalContext' 2>/dev/null)
event=$(printf '%s' "$d_out" | jq -r '.hookSpecificOutput.hookEventName' 2>/dev/null)
{ [ "$ctx" = "$(printf 'first\n\nsecond')" ] && [ "$event" = "SessionStart" ] && [ "$(printf '%s\n' "$d_out" | wc -l | tr -d ' ')" = 1 ]; } \
  && ok "two contexts joined into one JSON" || bad "two contexts joined into one JSON" "first, a blank line, second" "$d_out"

# Plain text next to JSON goes into the same context.
fake_hook "$local_dir/session-start/note.sh" note 'echo "plain text"'
dispatch session-start
ctx=$(printf '%s' "$d_out" | jq -r '.hookSpecificOutput.additionalContext' 2>/dev/null)
[ "$ctx" = "$(printf 'first\n\nplain text')" ] \
  && ok "plain text next to JSON — into the context" || bad "plain text next to JSON — into the context" "first + plain text" "$d_out"

# Without JSON plain text is printed as is.
fake_hook "$dproj/.harness/hooks/session-tree.sh" session-tree 'echo "line one"'
dispatch session-start
[ "$d_out" = "$(printf 'line one\nplain text')" ] \
  && ok "only plain text — as is" || bad "only plain text — as is" "two lines" "$d_out"

# A hook error is not a denial: the others run, exit 1 without JSON.
fake_hook "$dproj/.harness/hooks/session-tree.sh" session-tree 'echo "failure" >&2; exit 1'
dispatch session-start
{ [ "$d_code" -eq 1 ] && [ "$d_log" = "session-tree note " ] && [ "$d_out" = "plain text" ] && [ "$d_err" = "failure" ]; } \
  && ok "a hook error doesn't stop the slot" || bad "a hook error doesn't stop the slot" "exit 1, both hooks, stderr \"failure\"" "exit $d_code, log $d_log, output $d_out, stderr $d_err"

dispatch no-such-slot
[ "$d_code" -eq 1 ] && [[ "$d_err" == *"unknown slot"* ]] \
  && ok "unknown slot — error" || bad "unknown slot — error" "exit 1" "exit $d_code: $d_err"

# Real wiring: wholesale staging denied through the project dispatcher.
out=$(cd "$staging_repo" && printf '%s' "$(bash_ev 'git add -A')" | "$hooks/dispatch.sh" pre-bash 2>/dev/null)
case "$out" in
  *'"permissionDecision":"deny"'*) ok "the project's dispatch pre-bash denies git add -A" ;;
  *) bad "the project's dispatch pre-bash denies git add -A" "denial" "$out" ;;
esac
out=$(cd "$staging_repo/wt" && printf '%s' "$(bash_ev 'git add -A')" | "$hooks/dispatch.sh" pre-bash 2>/dev/null)
[ -z "$out" ] && ok "the project's dispatch pre-bash lets git add -A through in own worktree" \
  || bad "the project's dispatch pre-bash lets git add -A through in own worktree" "silence" "$out"

# --- lefthook ---------------------------------------------------------------
# The project's real `lefthook.yml` and `.harness/lefthook.yml` on a temporary
# repository. The binary is the project's (node_modules) or from PATH.
lefthook_bin=""
if [ -x "$root/node_modules/.bin/lefthook" ]; then
  lefthook_bin="$root/node_modules/.bin/lefthook"
elif command -v lefthook >/dev/null 2>&1; then
  lefthook_bin=$(command -v lefthook)
fi

# A copy of the project's git hook wiring and knowledge/ scripts into a temporary repository.
lefthook_copy() {  # <dir>
  mkdir -p "$1/.harness" "$1/knowledge"
  cp "$root/lefthook.yml" "$1/"
  cp "$root/.harness/lefthook.yml" "$1/.harness/"
  cp -R "$root/.harness/lefthook" "$1/.harness/"
  cp "$root/.rootstock.toml" "$1/"
  cp "$root"/knowledge/*.sh "$1/knowledge/"
}

answer() {  # <key> — a value from the `[answers]` table of the project's template record
  awk -v key="$1" '
    /^[ \t]*\[/ { inside = ($0 ~ /^[ \t]*\[answers\][ \t]*$/); next }
    inside && $0 ~ ("^" key "[ \t]*=") { v = substr($0, index($0, "=") + 1); gsub(/^[ \t\047"]+|[ \t\047"]+$/, "", v); print v; exit }
  ' "$root/.rootstock.toml"
}

echo
echo "pre-commit: closed questions archive"
# stage_fixed puts only staged files back into the index, and the closed questions
# archive is written by the script itself. A closure must go in one commit with the
# archive; a commit without a closure doesn't touch the archive.
if [ -z "$lefthook_bin" ]; then
  bad "lefthook in place" "node_modules/.bin/lefthook or lefthook in PATH" "missing"
else
  kidir=$(new_repo) || exit 1
  lefthook_copy "$kidir"
  questions=$(answer open_questions_path)
  closed="${questions%.md}-closed.md"
  mkdir -p "$kidir/$(dirname "$questions")"
  printf '# Open questions\n\n---\n\n## First question\n\nGist.\n\n---\n\n## Second question\n\nGist.\n' > "$kidir/$questions"
  git -C "$kidir" add -A
  git -C "$kidir" commit -q --no-verify -m base
  (cd "$kidir" && "$lefthook_bin" install >/dev/null 2>&1)

  # A commit without a closure: the archive doesn't appear and doesn't get into the commit.
  printf '\n---\n\n## Third question\n\nGist.\n' >> "$kidir/$questions"
  git -C "$kidir" add "$questions"
  git -C "$kidir" commit -q -m question >/dev/null 2>&1
  out=$(git -C "$kidir" show --name-only --format= HEAD | sort | tr '\n' ' ')
  { [ "$out" = "$questions " ] && [ ! -e "$kidir/$closed" ]; } \
    && ok "a commit without a closure doesn't touch the archive" \
    || bad "a commit without a closure doesn't touch the archive" "$questions" "$out"

  # A closure: both files in one commit, no leftovers in the worktree.
  perl -i -pe 's/^## First question$/## ~~First question~~ — closed 2026-10-08/' "$kidir/$questions"
  git -C "$kidir" add "$questions"
  git -C "$kidir" commit -q -m close >/dev/null 2>&1
  out=$(git -C "$kidir" show --name-only --format= HEAD | sort | tr '\n' ' ')
  rest=$(git -C "$kidir" status --porcelain)
  want=$(printf '%s\n%s\n' "$closed" "$questions" | sort | tr '\n' ' ')
  { [ "$out" = "$want" ] && [ -z "$rest" ]; } \
    && ok "closure and archive — one commit" \
    || bad "closure and archive — one commit" "$want, a clean worktree" "$out / $rest"

  # Tables of contents: the job calls `knowledge/index.sh --stage` — shared in the
  # catalog form, the project's in the monolith form. The stand-in script writes its
  # argument to a file and stages it itself, as the real one must.
  printf '#!/usr/bin/env bash\nprintf "%%s\\n" "$*" > knowledge/toc.md\n[ "${1:-}" != --stage ] || git add knowledge/toc.md\n' > "$kidir/knowledge/index.sh"
  chmod +x "$kidir/knowledge/index.sh"
  git -C "$kidir" add knowledge/index.sh
  git -C "$kidir" commit -q --no-verify -m index
  printf '\n---\n\n## Fourth question\n\nGist.\n' >> "$kidir/$questions"
  git -C "$kidir" add "$questions"
  git -C "$kidir" commit -q -m toc >/dev/null 2>&1
  out=$(git -C "$kidir" show --name-only --format= HEAD | sort | tr '\n' ' ')
  want=$(printf '%s\n%s\n' knowledge/toc.md "$questions" | sort | tr '\n' ' ')
  toc=$(git -C "$kidir" show HEAD:knowledge/toc.md 2>/dev/null)
  { [ "$out" = "$want" ] && [ "$toc" = --stage ]; } \
    && ok "pre-commit calls knowledge/index.sh --stage, the toc in the same commit" \
    || bad "pre-commit calls knowledge/index.sh --stage, the toc in the same commit" "$want, --stage" "$out / $toc"
fi

echo
echo "pre-push: push files"
# The branch is created with --no-track and is behind main, into which a knowledge/
# file was merged after branching. Pushing one document must not wake knowledge-tests,
# and pushing an own knowledge/ file must. All that matters here is whether the job is skipped.
if [ -n "$lefthook_bin" ]; then
  ppdir=$(new_tmp)
  git init -q --bare "$ppdir/origin.git"
  git init -q -b main "$ppdir/w"
  pp=$ppdir/w
  git -C "$pp" config user.email t@t
  git -C "$pp" config user.name t
  lefthook_copy "$pp"
  echo a > "$pp/knowledge/a.md"
  git -C "$pp" add -A
  git -C "$pp" commit -q --no-verify -m base
  git -C "$pp" remote add origin "$ppdir/origin.git"
  git -C "$pp" push -q --no-verify origin main
  git -C "$pp" switch -q -c docs --no-track
  echo d > "$pp/d.md"
  git -C "$pp" add d.md
  git -C "$pp" commit -q --no-verify -m docs
  git -C "$pp" switch -q main
  echo b > "$pp/knowledge/b.md"
  git -C "$pp" add knowledge/b.md
  git -C "$pp" commit -q --no-verify -m knowledge
  git -C "$pp" push -q --no-verify origin main
  git -C "$pp" switch -q docs

  # The state at the moment of `git push` of a new branch: HEAD is docs, no upstream.
  push_job() { (cd "$pp" && "$lefthook_bin" run pre-push --job knowledge-tests --no-tty --colors off </dev/null 2>&1); }

  out=$(push_job)
  [[ $out == *"knowledge-tests (skip)"* ]] \
    && ok "docs of a lagging branch don't wake knowledge-tests" \
    || bad "docs of a lagging branch don't wake knowledge-tests" "knowledge-tests (skip)" "$out"

  # Control: without our own list lefthook wakes knowledge-tests with someone else's
  # knowledge/b.md from main. If it stops doing so, the test above proves nothing.
  grep -v '^  files: .*push-files\.sh$' "$root/.harness/lefthook.yml" > "$pp/.harness/lefthook.yml"
  out=$(push_job)
  [[ $out != *"knowledge-tests (skip)"* ]] \
    && ok "control: the built-in list wakes knowledge-tests with someone else's knowledge/" \
    || bad "control: the built-in list wakes knowledge-tests with someone else's knowledge/" "no skip" "$out"
  cp "$root/.harness/lefthook.yml" "$pp/.harness/"

  echo c > "$pp/knowledge/c.md"
  git -C "$pp" add knowledge/c.md
  git -C "$pp" commit -q --no-verify -m own
  out=$(push_job)
  [[ $out != *"knowledge-tests (skip)"* ]] \
    && ok "an own knowledge/ file wakes knowledge-tests" \
    || bad "an own knowledge/ file wakes knowledge-tests" "no skip" "$out"
fi

echo
printf 'total: %d passed, %d failed\n' "$pass" "$fail"
[ "$fail" -eq 0 ]
