#!/usr/bin/env bash
# Test of update.sh on temporary repositories: updating a project via rootstock.
# rootstock is replaced by a stub that edits files and prints conflicts the way
# the real one does: the test checks the glue (commit, output, exit codes).
set -uo pipefail

# Under a git hook the environment has GIT_DIR and others: git would look elsewhere.
while IFS='=' read -r name _; do
  case "$name" in GIT_*) unset "$name" ;; esac
done < <(env)
export GIT_AUTHOR_NAME=bot GIT_AUTHOR_EMAIL=bot@example.invalid
export GIT_COMMITTER_NAME=bot GIT_COMMITTER_EMAIL=bot@example.invalid

script="$(cd "${BASH_SOURCE[0]%/*}/.." && pwd)/update.sh"
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

pass=0
fail=0
ok() { pass=$((pass + 1)); printf '  ok   %s\n' "$1"; }
bad() { fail=$((fail + 1)); printf '  FAIL %s\n     expected: %s\n     got: %s\n' "$1" "$2" "${3:-<empty>}"; }
same() { if [[ $3 == "$2" ]]; then ok "$1"; else bad "$1" "$2" "$3"; fi; }

echo "== update.sh"

"$script" >/dev/null 2>&1
same "no tag: code 2" 2 "$?"
"$script" v1.0.0 v2.0.0 >/dev/null 2>&1
same "two tags: code 2" 2 "$?"

# A project on rootstock v1.0.0.
new_project() {
  local dir="$tmp/$1"
  git init -q -b main "$dir"
  mkdir -p "$dir/.harness/update" "$dir/sub"
  cp "$script" "$dir/.harness/update/"
  printf 'schema = 1\nversion = "v1.0.0"\n' >"$dir/.rootstock.toml"
  printf 'a\n' >"$dir/a.txt"
  printf 'keep\n' >"$dir/sub/keep.txt"
  git -C "$dir" add -A && git -C "$dir" commit -qm init
  printf '%s' "$dir"
}

stubs="$tmp/stubs"
mkdir -p "$stubs"
log="$tmp/calls"

# rootstock: like the real one, update requires a clean tree and the project root;
# prints conflicts as `<kind> <path>` and exits with code 2. $RS_CODE is the exit
# code (default 0, no conflicts); at 1 it leaves files alone.
cat >"$stubs/rootstock" <<EOF
#!/usr/bin/env bash
echo "rootstock \$* @ \$(basename "\$PWD")" >>"$log"
[ -f .rootstock.toml ] || { echo "error: no record" >&2; exit 1; }
[ -z "\$(git status --porcelain)" ] || { echo "error: dirty" >&2; exit 1; }
code=\${RS_CODE:-0}
[ "\$code" = 1 ] && { echo "error: boom" >&2; exit 1; }
printf 'schema = 1\nversion = "v2.0.0"\n' >.rootstock.toml
echo a2 >a.txt
if [ "\$code" = 2 ]; then
  echo "markers a.txt"
  echo "deleted-by-project gone.txt"
  echo "markers a.txt"
fi
exit "\$code"
EOF
chmod +x "$stubs/rootstock"

# Clean update: one commit, no conflicts.
proj=$(new_project clean)
: >"$log"
out=$(cd "$proj" && ROOTSTOCK="$stubs/rootstock" "$script" v2.0.0 2>/dev/null)
same "clean: code 0" 0 "$?"
same "clean: no conflicts" "" "$out"
same "clean: called from the project root" "rootstock update --to v2.0.0 --defaults --trust @ clean" "$(cat "$log")"
same "clean: one commit" "Update harness to v2.0.0" "$(git -C "$proj" log --format=%s main~1..main)"
same "clean: commit has the record and the edit" "$(printf '.rootstock.toml\na.txt')" \
  "$(git -C "$proj" diff --name-only main~1 main)"
same "clean: tree is clean" "" "$(git -C "$proj" status --porcelain)"
same "clean: identity" "bot bot@example.invalid" "$(git -C "$proj" log -1 --format='%an %ae')"

# Conflicts (code 2): the update is committed, conflicts without duplicates.
proj=$(new_project conflict)
: >"$log"
out=$(cd "$proj/sub" && RS_CODE=2 ROOTSTOCK="$stubs/rootstock" "$script" v2.0.0 2>/dev/null)
same "conflict: code 0" 0 "$?"
same "conflict: from a subdirectory, rootstock runs at the root" "rootstock update --to v2.0.0 --defaults --trust @ conflict" "$(cat "$log")"
same "conflict: list without duplicates" "$(printf 'deleted-by-project gone.txt\nmarkers a.txt')" "$out"
same "conflict: one commit" "Update harness to v2.0.0" "$(git -C "$proj" log --format=%s main~1..main)"
same "conflict: tree is clean" "" "$(git -C "$proj" status --porcelain)"

# rootstock failure: code 1, no commit.
proj=$(new_project fail)
(cd "$proj" && RS_CODE=1 ROOTSTOCK="$stubs/rootstock" "$script" v2.0.0 >/dev/null 2>&1)
same "failure: code 1" 1 "$?"
same "failure: no update commit" 0 "$(git -C "$proj" log --format=%s | grep -c 'Update harness')"

printf '%d ok, %d failed\n' "$pass" "$fail"
((fail == 0))
