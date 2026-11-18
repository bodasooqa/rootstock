#!/bin/sh
# pre-push: the files this push sends, for job globs.
#
# POSIX sh, not bash: lefthook calls it as `sh ./…` (on Windows `files:` runs
# without a shell), and on Linux `sh` is dash.
#
# Our own list instead of lefthook's built-in one. Without an upstream (first
# push of a branch created with --no-track) lefthook uses
# `git diff --name-only HEAD origin/main`: a comparison of two tips, not from the
# common ancestor. A branch behind main got other people's changes merged into
# main after it branched off, and expensive jobs ran on a docs-only push.
#
# Here the base is what origin already has for this branch (`@{push}`), and
# without it, the common ancestor with origin/main. Three-dot comparison: only
# what appeared in the branch.
set -u

if base=$(git rev-parse --verify --quiet '@{push}'); then
  :
elif base=$(git merge-base HEAD origin/main 2>/dev/null); then
  :
else
  # Nothing to compare against: an extra check is better than a silently skipped one.
  git ls-files
  exit 0
fi

git diff --name-only "$base"...HEAD --
