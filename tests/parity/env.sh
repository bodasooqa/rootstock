# shellcheck shell=sh
# Shared setup of the parity scripts, sourced by render.sh and update.sh.
COPIER=$(command -v "${COPIER:-copier}") || { echo "copier not found" >&2; exit 1; }
HARNESS=${HARNESS:-https://github.com/bodasooqa/harness}
WORK=${WORK:-$(mktemp -d)}
if [ -z "${PARITY_PYTHON:-}" ]; then
  # The interpreter of the `copier` script has PyYAML; `#!/usr/bin/env python3`
  # names it through env.
  PARITY_PYTHON=$(sed -n '1s/^#!//p' "$COPIER")
  case $PARITY_PYTHON in
    */env\ *) PARITY_PYTHON=$(command -v "${PARITY_PYTHON#*env }") ;;
  esac
fi
PY=$PARITY_PYTHON

# Both tools run under the same git setup, whatever the user's config says:
# an empty global config (no merge.conflictStyle, no advice) and a fixed author.
: > "$WORK/gitconfig"
GIT_CONFIG_GLOBAL=$WORK/gitconfig
GIT_CONFIG_NOSYSTEM=1
GIT_AUTHOR_NAME=parity GIT_AUTHOR_EMAIL=parity@localhost
GIT_COMMITTER_NAME=parity GIT_COMMITTER_EMAIL=parity@localhost
export GIT_CONFIG_GLOBAL GIT_CONFIG_NOSYSTEM GIT_AUTHOR_NAME GIT_AUTHOR_EMAIL \
  GIT_COMMITTER_NAME GIT_COMMITTER_EMAIL
git config --global advice.detachedHead false
echo "copier: $("$COPIER" --version); python: $PY; work: $WORK" >&2
