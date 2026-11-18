#!/usr/bin/env bash
# Picks the harness version to update to. Called from
# `.github/workflows/harness-update.yml`; the logic lives here so it can be
# tested without GitHub (`.harness/update/tests/next-version.sh`).
#
# Usage:
#   next-version.sh [--want <tag>] <current> < tags
#       Source tags, one per line, as names (`v0.2.0`) or refs
#       (`refs/tags/v0.2.0`). Current is `version` from `.rootstock.toml`:
#       a tag `vX.Y.Z` or git describe output `vX.Y.Z-N-g<hash>` (a project
#       created not from a tag; its version counts as the tag it grew from).
#       Prints `key=value` lines for $GITHUB_OUTPUT:
#         current=vX.Y.Z   project version;
#         next=vA.B.C      version to update to, empty if there is nothing to update;
#         major=true       A is greater than X (otherwise false).
#       Without --want, takes the latest `vX.Y.Z` tag by semver, skipping
#       pre-releases (`v1.0.0-rc.1`). With --want, that tag, pre-release too; it
#       must be among the tags.
#   next-version.sh --lt <a> <b>
#       Code 0 if version a is less than b, otherwise 1.
#
# Code 2: argument error, unrecognized version, or --want without such a tag.
set -euo pipefail

die() {
  printf 'next-version.sh: %s\n' "$1" >&2
  exit 2
}

# Parses a version into globals ver_major, ver_minor, ver_patch, ver_pre.
# Code 1 if the string is not a version vX.Y.Z[-pre-release].
parse() {
  local re='^v([0-9]+)\.([0-9]+)\.([0-9]+)(-([0-9A-Za-z.-]+))?$'
  [[ $1 =~ $re ]] || return 1
  ver_major=$((10#${BASH_REMATCH[1]}))
  ver_minor=$((10#${BASH_REMATCH[2]}))
  ver_patch=$((10#${BASH_REMATCH[3]}))
  ver_pre=${BASH_REMATCH[5]}
}

# Code 0 if version $1 is less than $2 by semver. Both already checked by parse.
# A pre-release is less than the release of the same X.Y.Z; two pre-releases are
# compared as versions (`sort -V`): exact semver order isn't needed for them here.
less() {
  local a_major a_minor a_patch a_pre
  parse "$1"
  a_major=$ver_major a_minor=$ver_minor a_patch=$ver_patch a_pre=$ver_pre
  parse "$2"
  if ((a_major != ver_major)); then
    ((a_major < ver_major)) && return 0 || return 1
  fi
  if ((a_minor != ver_minor)); then
    ((a_minor < ver_minor)) && return 0 || return 1
  fi
  if ((a_patch != ver_patch)); then
    ((a_patch < ver_patch)) && return 0 || return 1
  fi
  if [[ -n $a_pre && -z $ver_pre ]]; then
    return 0
  fi
  if [[ -z $a_pre || $a_pre == "$ver_pre" ]]; then
    return 1
  fi
  [[ $(printf '%s\n%s\n' "$a_pre" "$ver_pre" | sort -V | head -n 1) == "$a_pre" ]]
}

if [[ ${1:-} == --lt ]]; then
  [[ $# -eq 3 ]] || die "--lt expects two versions"
  parse "$2" || die "not a version: $2"
  parse "$3" || die "not a version: $3"
  less "$2" "$3"
  exit
fi

want=""
if [[ ${1:-} == --want ]]; then
  [[ -n ${2:-} ]] || die "--want expects a tag"
  want=$2
  shift 2
fi
[[ $# -eq 1 ]] || die "expected one current version (version from .rootstock.toml)"

# The git describe tail `-N-g<hash>` is commits after the tag: same version.
current=$1
if [[ $current =~ ^(.+)-[0-9]+-g[0-9a-f]+$ ]]; then
  current=${BASH_REMATCH[1]}
fi
parse "$current" || die "current version not recognized: $1 (expected vX.Y.Z or vX.Y.Z-N-g<hash>)"
current_major=$ver_major

next=""
while IFS= read -r tag; do
  tag=${tag#refs/tags/}
  [[ -n $tag ]] || continue
  parse "$tag" || continue
  if [[ -n $want ]]; then
    [[ $tag == "$want" ]] && next=$tag
    continue
  fi
  [[ -z $ver_pre ]] || continue
  if [[ -z $next ]] || less "$next" "$tag"; then
    next=$tag
  fi
done

if [[ -n $want && -z $next ]]; then
  die "tag $want is not among the source tags"
fi

major=false
if [[ -n $next ]] && less "$current" "$next"; then
  parse "$next"
  ((ver_major > current_major)) && major=true
else
  next=""
fi

printf 'current=%s\nnext=%s\nmajor=%s\n' "$current" "$next" "$major"
