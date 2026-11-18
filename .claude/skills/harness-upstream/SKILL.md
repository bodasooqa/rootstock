---
name: harness-upstream
description: Use when the harness itself needs a change — a managed file of the harness template (.harness/ outside .harness/local/, template skills and agents, .pi/, knowledge/ scripts, harness*.yml workflows) is wrong or missing something. Opens a PR in the template repository from this project's session, runs the template's checks, and brings the release back through the update PR.
---

# Improving the harness — a PR to the template

First check `.harness/local/skills/harness-upstream.md`: if the file exists, read it before starting work. It is the project addendum — where the template clone is, the project's own check steps; where it disagrees with the text below, the addendum wins.

The project's harness comes from the template (`.rootstock.toml`: where from and which version). Managed files are updated in all projects at once, so they are not edited in the project. An edit to a managed file here will be lost or become a conflict at the next update. An improvement goes into the template and comes back with a release.

## 1. Shared or the project's own

| What is needed | Where |
|---|---|
| A rule, check, or hook that holds for any project | A PR to the template — this skill |
| This project's domain, invariants, paths, commands | the project's extension points: `.harness/local/` (hooks by slot, agent and skill addenda), `AGENTS.md`, its own skills and jobs alongside. Their list is `.harness/local/README.md` |
| An extension point is missing | That is a template improvement too: the PR adds the point, the project uses it |

The project's starter files (`AGENTS.md`, `CLAUDE.md`, `README.md`, `lefthook.yml`, `biome.json`, `.github/workflows/ci.yml`, the code skeleton, documents) belong to the project and are edited here. The full list is `starter` in the template's `rootstock.toml`.

The template is public: the PR contains no domain, paths, names, or secrets of the project, and any example is neutral.

## 2. Where to work

The template repository is `source` from `.rootstock.toml`. Work happens in its clone, outside the project tree. If the owner already has a clone, ask for its location once: create your own worktree (`git worktree add`) with your own branch from origin/main. If there is no clone, `gh repo clone` into a directory next to the project. The project tree is not touched.

Sources are in `template/`: `.jinja` files are rendered with the answers, shared pieces are in `template/_partials/`. Questions and the lists of starter and excluded files are in `rootstock.toml`. The design is the template's `docs/design.md`.

## 3. The edit

1. Reproduce. The bug — on the project or on a template layout, before the edit: "here is the command, here is the wrong result".
2. A test where this part of the template is checked:
   - hooks — `template/.harness/hooks/tests/run.sh`;
   - the Pi shim — `pi.test.ts`;
   - `knowledge/` scripts — `template/knowledge/tests/`;
   - update delivery — `template/.harness/update/tests/`;
   - layouts and `rootstock update` — `scripts/check-layouts.sh`, `scripts/check-update.sh`.

   The test goes red without the edit and green with it.
3. The edit. Template answers (`rust`, `frontend`, document paths…) — via Jinja, not via a special case for one project.

## 4. Checks

From the template root, tools as per the template README:

- `scripts/check-layouts.sh` — all layouts, or `--only <layout>` for the affected ones, but no fewer than `full`;
- `scripts/check-update.sh` — if `rootstock.toml`, starter files, or update delivery were touched;
- `shellcheck -x scripts/*.sh`, `actionlint .github/workflows/*.yml`.

## 5. PR

- Title — `template: …`. Body: what and why, how it was found (without the domain), how it was checked.
- Release kind label:
  - `release:patch` — a fix;
  - `release:minor` — a new capability compatible with the previous one (a new entry point, a new extension point);
  - `release:major` — a path renamed, a question changed, projects need a migration. Plus a note with migration steps in the template's `CHANGELOG.md`; the tag for such a release is set manually;
  - `release:none` — a change to the template repository itself, not delivered to projects.
- The template owner merges. The patch and minor tag is set automatically after green CI.

## 6. Back in the project

- Until there is a release, the managed file is not edited in the project. If a workaround is urgent — only through an extension point, and it is removed with the release.
- After the tag, the PR `harness/update-<tag>` arrives on its own: the `harness-update` workflow runs once a day. Right away — `gh workflow run harness-update -f version=<tag>`. A PR without conflicts and not major is merged automatically on green CI.
- A PR labeled `harness:conflict` or `harness:major` is resolved by hand in its own worktree: conflict markers, migration steps from `CHANGELOG.md`.
- Remove your worktree of the template clone and the branch after the merge.

## What not to do

- Don't edit a managed file in the project "until the template is released".
- Don't pull into the template what holds only for this project.
- Don't work around a template check that gets in the project's way by editing the project. A check that gets in the way is itself a PR candidate.
