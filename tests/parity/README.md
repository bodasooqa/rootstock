# Copier parity checks

Acceptance tools for Stage 2 of `docs/design.md`, not tests: they need Copier 9.19 and network or local clones of harness and screener, so CI does not run them.

- `render.sh <rootstock-binary> <tag>` renders harness `<tag>` with Copier and with rootstock for every valid combination of the answers `rust`, `frontend`, `monorepo`, `linear`, `telegram`, `orca` and `doc_form` (64 for harness v0.4.5). Copier takes a clean clone at the tag. rootstock takes the same tag converted by `import-copier` in the working tree, through `new --version HEAD`, so the record points at the tag's commit. The trees are compared entry by entry: kind, executable bit, symlink target, bytes. The record's answers and commit are compared with `.copier-answers.yml`.
- `update.sh <rootstock-binary> <project-commit> <tag>` checks out the project (screener by default) at `<project-commit>` twice and points `_src_path` at a local clone of harness. One copy is updated with `copier update --vcs-ref <tag>`, the other with `rootstock import-copier`, a commit, and `rootstock update --to <tag>`. The trees are compared the same way. Copier's conflict labels (`before updating`, `after updating`) are first rewritten to rootstock's (`project`, `template <tag>`), which `docs/design.md` sets.

Both run tasks and hooks with `PATH=/usr/bin:/bin`, so harness's `openspec init` task is the same no-op on both sides. Both tools also run with an empty global git config (`GIT_CONFIG_GLOBAL`, `GIT_CONFIG_NOSYSTEM=1`): a user's `merge.conflictStyle = diff3` would otherwise change Copier's conflicts and show a false difference. Shared setup is in `env.sh`; the comparison is `compare.py`, run with Copier's Python.

Both print one line per difference and exit 1 if there is any. `update.sh` also prints a `listed:` line for every file whose Copier conflict labels it rewrote: those are differences the design sets, listed for the stage report, not failures. Copier's unmerged index entries (`UU` in `git status`) are not compared, since `.git` is skipped.

Environment:

| Variable | Default |
|---|---|
| `COPIER` | `copier` on `PATH` |
| `HARNESS` | `https://github.com/bodasooqa/harness` |
| `SCREENER` | `https://github.com/bodasooqa/screener` (`update.sh` only) |
| `PARITY_PYTHON` | the interpreter of the `copier` script (it has PyYAML) |
| `WORK` | a new `mktemp -d` directory, kept for inspection |

Stage 2 acceptance ran:

```sh
cargo build --release
tests/parity/render.sh target/release/rootstock v0.4.5
tests/parity/update.sh target/release/rootstock f274eefc v0.4.5   # screener on harness v0.3.3
```
