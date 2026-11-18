---
name: change-finish
description: Use when the code of an OpenSpec change is written and the change has to be brought to a finished state — verification, reviewers, archive, commit, push, PR; and, after the PR is merged, the second pass that cleans the session's worktree and branches up.
---

# Finishing a change

First check `.harness/local/skills/change-finish.md`: if the file exists, read it before starting work. It is the project addendum — the project's own checks, its own steps before the PR, who merges the PR; where it disagrees with the text below, the addendum wins.

Between "the code is written" and "it's in `main`, and the session is cleaned up" lies a tail that breaks more quietly than anything else: checks are run by the executor rather than by whoever is accountable; checkboxes and spec sync get forgotten; branches and worktrees pile up, and the main checkout falls behind `main` — and no check says so.

This skill is about one change: from finished code to a clean worktree. Stage acceptance is `stage-acceptance`; it is about something else and stays where it is.

Two passes: the first gets to the PR, the second runs after the merge.

## Pass 1 — before the PR

**1. Clean check. The main session runs it.** This is the only full local run: the executor checks only what was affected, and pre-push doesn't run the heavy stuff. After the PR, CI will run the same. The executor's report doesn't count as proof: different build slots give different results on the same tree, and a green executor report has more than once turned into red tests in the main session.

```
cargo clean -p rootstock
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test
```

The clean is not a formality: without it the run may show the result of a build made before the edit.

An error like "no such field" or "trait not implemented" where everything is in place means not a defect but someone else's build: someone else shares the build directory with this worktree. One command checks it — the same run with `CARGO_TARGET_DIR` pointing to its own directory; if that is green, fix the setup, not the code.

**2. Reviewers.** Run the reviewer subagents: `rust-reviewer` — on the Rust part; `spec-auditor` — if product behavior changed. Each finding is either fixed or named as a deviation with a reason; none is silently dropped.

A reviewer that finds a regression thereby cancels step 1: after the fixes, the checks are rerun in full.

**3. The change's own loose ends.**

- checkboxes in `openspec/changes/<name>/tasks.md` are ticked by fact, not by intention;
- the delta spec is synced with `openspec/specs/` (the OpenSpec sync command), `openspec validate --specs` and `openspec validate <name> --strict` are green;
- open questions the change resolved are closed, decisions and defects found are recorded — the `knowledge-note` skill (decisions — in `knowledge/decisions.md`, lessons — in `knowledge/lessons.md`);
- `knowledge/due.sh --overdue` is green.

**4. Archive** — the OpenSpec archive command.

**5. Divergence from `main` is resolved before the PR.** `git fetch origin --prune`, then `git merge-tree --write-tree origin/main HEAD` — it plays the merge dry and names the files that will conflict. Empty — move on. Not empty — bring in `main` **by merge, not rebase**: a rebase makes you resolve the same conflict once per each of your commits, and the branch will land in `main` as a squash anyway.

After resolving — always search the whole tree for leftover markers:

```
git grep -n '^[[:space:]]*\(<<<<<<<\|=======$\|>>>>>>>\)' -- . ':!node_modules'
```

Resolving a conflict and removing the markers are different things: markers left behind go straight into the commit. Leading whitespace before a marker is allowed on purpose: after an edit inside the block being resolved, a marker may end up indented, and a pattern with `^` right before the marker doesn't see it. Then rerun the checks of step 1: the merge brought in someone else's code, and what was green before it says nothing about after.

**6. Commit and push.** Files are named one by one. A branch on origin with the same name but diverged means a previous PR: push under a new name, not with force — a force push over the branch of a merged PR destroys history that people may still be reading.

**7. PR.** The title names the change. The body: what changed and why, how it was checked, what remains unresolved and where it is recorded. Wait for CI (`gh pr checks <number> --watch`): red CI means going back to step 1, not a message to the owner. If the hook names a divergence with the branch of a neighboring worktree, name it in the PR as well: a merge order is needed, not a surprise at the second button. Who presses the merge button is a project rule (the skill addendum or `AGENTS.md`); if it isn't recorded, merging is up to the owner, and they get the link.

## Pass 2 — after the merge

Runs when the PR is merged.

1. `git fetch origin --prune`, then `git pull` in the main checkout: otherwise it stays on the old commit and misleads the next session.
2. This session's own worktree and its local branch are removed without asking — the work is in `main` (`orca worktree rm`).
3. The remote branch and other worktrees — only with the owner's answer. Another worktree may be in use by a live session, and removing it would take its work with it.
4. `knowledge/due.sh --overdue` and the session leftovers hook (`.harness/hooks/session-leftovers.sh`, can also be run by hand) are both silent. If the hook still names a leftover, the pass is not finished.

## What not to do

- Don't trust the executor's report instead of your own run. This is not distrust of the executor: different build slots give different results on the same tree.
- Don't press merge on a PR unless the project explicitly allowed it.
- Don't remove someone else's worktree and don't force-push someone else's branch.
- Don't archive a change whose `tasks.md` still has unfinished items — either finish them or name what's unfinished in the PR.
- Don't postpone the entries in `knowledge/`: what isn't recorded in this pass will never be recorded, and the hook won't remind you — it sees branches and worktrees, not unwritten lessons.
