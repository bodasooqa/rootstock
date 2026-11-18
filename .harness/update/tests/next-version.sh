#!/usr/bin/env bash
# Test of next-version.sh: picking the latest tag, pre-releases, describe output,
# major version, --want and the --lt comparison.
set -uo pipefail

script="$(cd "${BASH_SOURCE[0]%/*}/.." && pwd)/next-version.sh"
pass=0
fail=0

ok() { pass=$((pass + 1)); printf '  ok   %s\n' "$1"; }
bad() { fail=$((fail + 1)); printf '  FAIL %s\n     expected: %s\n     got: %s\n' "$1" "$2" "${3:-<empty>}"; }

# Case: name, space-separated tags, script arguments, expected output as one
# space-separated line.
expect() {
  local name=$1 tags=$2 want=$3 got
  shift 3
  got=$(tr ' ' '\n' <<<"$tags" | "$script" "$@" 2>&1 | tr '\n' ' ')
  got=${got% }
  if [[ $got == "$want" ]]; then ok "$name"; else bad "$name" "$want" "$got"; fi
}

# Case with expected exit code 2.
expect_error() {
  local name=$1 tags=$2 code
  shift 2
  tr ' ' '\n' <<<"$tags" | "$script" "$@" >/dev/null 2>&1
  code=$?
  if [[ $code -eq 2 ]]; then ok "$name"; else bad "$name" "code 2" "code $code"; fi
}

# Comparison: name, a, b, expected code.
expect_lt() {
  local name=$1 code
  "$script" --lt "$2" "$3" >/dev/null 2>&1
  code=$?
  if [[ $code -eq $4 ]]; then ok "$name"; else bad "$name" "code $4" "code $code"; fi
}

tags="v0.1.0 v0.1.1 v0.2.0 v0.10.0 v0.9.3"

echo "== next-version.sh"
expect "latest tag by semver, not by string" "$tags" \
  "current=v0.1.0 next=v0.10.0 major=false" v0.1.0
expect "refs/tags/ refs and junk tags" "refs/tags/v0.1.0 refs/tags/v0.1.1 latest release-1" \
  "current=v0.1.0 next=v0.1.1 major=false" v0.1.0
expect "already latest: nothing to update" "$tags" \
  "current=v0.10.0 next= major=false" v0.10.0
expect "project newer than the latest tag: nothing to update" "$tags" \
  "current=v0.11.0 next= major=false" v0.11.0
expect "describe: commits after the tag count as the tag version" "$tags" \
  "current=v0.9.3 next=v0.10.0 major=false" v0.9.3-4-g4cf763c
expect "describe of the latest tag: nothing to update" "$tags" \
  "current=v0.10.0 next= major=false" v0.10.0-1-gabc1234
expect "a pre-release is not taken as latest" "v0.1.0 v0.1.1 v1.0.0-rc.1" \
  "current=v0.1.0 next=v0.1.1 major=false" v0.1.0
expect "major version" "v0.1.0 v0.1.1 v1.0.0" \
  "current=v0.1.1 next=v1.0.0 major=true" v0.1.1
expect "major from 1.x" "v1.4.2 v2.0.0" \
  "current=v1.4.2 next=v2.0.0 major=true" v1.4.2
expect "minor is not major" "v1.4.2 v1.5.0" \
  "current=v1.4.2 next=v1.5.0 major=false" v1.4.2
expect "no tags: nothing to update" "" \
  "current=v0.1.0 next= major=false" v0.1.0
expect "--want: tag below the latest" "$tags" \
  "current=v0.1.0 next=v0.2.0 major=false" --want v0.2.0 v0.1.0
expect "--want: explicit pre-release" "v0.1.0 v1.0.0-rc.1" \
  "current=v0.1.0 next=v1.0.0-rc.1 major=true" --want v1.0.0-rc.1 v0.1.0
expect "--want: not newer than current, nothing to update" "$tags" \
  "current=v0.2.0 next= major=false" --want v0.1.1 v0.2.0
expect "from a pre-release to the release of the same version" "v1.0.0-rc.1 v1.0.0" \
  "current=v1.0.0-rc.1 next=v1.0.0 major=false" v1.0.0-rc.1
expect_error "--want: no such tag" "$tags" --want v0.3.0 v0.1.0
expect_error "current is not a version (hash without a tag)" "$tags" 4cf763c
expect_error "no current" "$tags"
expect_error "extra argument" "$tags" v0.1.0 v0.1.1

expect_lt "--lt: patch" v0.1.0 v0.1.1 0
expect_lt "--lt: equal" v0.1.1 v0.1.1 1
expect_lt "--lt: greater" v0.2.0 v0.1.9 1
expect_lt "--lt: numbers, not strings" v0.9.0 v0.10.0 0
expect_lt "--lt: pre-release less than release" v1.0.0-rc.1 v1.0.0 0
expect_lt "--lt: release not less than pre-release" v1.0.0 v1.0.0-rc.1 1
expect_lt "--lt: rc.2 less than rc.10" v1.0.0-rc.2 v1.0.0-rc.10 0
expect_lt "--lt: not a version, code 2" v1 v0.1.0 2

printf '\nnext-version.sh: %d ok, %d failed\n' "$pass" "$fail"
[[ $fail -eq 0 ]]
