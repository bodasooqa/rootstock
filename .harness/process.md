# Process rules

Shared working rules from the [harness](https://github.com/bodasooqa/harness) template. This file is managed: it is updated by `rootstock update` and is not edited by hand. The project writes its own rules in the root `AGENTS.md` (domain, stack, package boundaries, sources of truth, definitions) and in `.harness/local/`. `AGENTS.md` is read by all agents; `CLAUDE.md` next to it only includes it and this file for Claude Code.

## Language

All artifacts, code comments, doc comments, error messages, commit messages, PR texts, and the knowledge journal are written in English. Agents talk to the owner in Russian.

## Three places for documentation

The spec (`docs/design.md`, revisions journal — `docs/design-revisions.md`) — requirements and the domain. `openspec/specs/` — implemented behavior by capability, derived from the spec. `knowledge/` — decisions with reasons, lessons, quirks of external APIs, open questions. Don't mix them; the "where to put it" rule — `knowledge/README.md` and the `knowledge-note` skill.

The spec is the file `docs/design.md`, the revision journal is `docs/design-revisions.md`: what changed and why, one section per version. The folded part of the spec carries the current requirements with version marks "(1.N)". Revisions not yet folded into it are listed in the spec header: for them the journal section is normative. Nobody reads whole files: the needed paragraph is found by headings (`grep -n '^## \|^### ' docs/design.md docs/design-revisions.md`) and read as a line range.

When in doubt about behavior — read the section, don't recall it.

The root `README.md` is not a fourth place but a showcase: what the system does, which stage it is at, how to run it, where things live. It must stay current and is edited in the same commit as what it describes: a package added or removed — the repository map; a run command or config key changed — the quick start.

## Workflow

Work goes through OpenSpec: propose → apply → archive. Use `/opsx:propose`, `/opsx:apply`, and `/opsx:archive` in Claude Code, `/opsx-propose`, `/opsx-apply`, and `/opsx-archive` in Pi, or `$openspec-propose`, `$openspec-apply-change`, and `$openspec-archive-change` in Codex. Capability and change names are kebab-case in English. Artifact rules and project context are in `openspec/config.yaml`.

One stage — several changes. A change name starts with the stage number: `stage3-<name>`. A stage is closed when all its changes are archived, the acceptance criterion from the spec's stages section is met (the `stage-acceptance` skill), and the stage table `knowledge/stages.md` shows it as `closed`. A change of the next stage does not start before that — and does not start before grooming of open questions (the `stage-grooming` skill, the list is printed by `knowledge/due.sh`).

A requirement that is not in the spec cannot be in a proposal. If something is missing — stop and ask.

The split with superpowers: **OpenSpec owns what is written down, superpowers owns how we work.**

- Before propose — `brainstorming`, so the proposal is not made up. Its terminal step ("invoke `writing-plans`") reads here as "move on to propose"; for the harness — as "lay out the plan in chat and record the decision in `knowledge/decisions.md`".
- On bugs — `systematic-debugging`. TDD is an executor rule, written in their prompts.
- From finished code to PR and cleanup after merge — the `change-finish` skill: full check run, reviewers, archiving, PR. It is the pre-completion check; `verification-before-completion`, `requesting-code-review`, `finishing-a-development-branch` are not invoked separately.
- `writing-plans` and `executing-plans` are not used: the plan lives in `openspec/changes/<name>/tasks.md`, we don't keep a second plan.
- Writing to `knowledge/` — the `knowledge-note` skill.

**The harness lives outside OpenSpec.** Skills, subagents, hooks, lefthook, CI, `knowledge/`, `AGENTS.md` and `CLAUDE.md` do not go through OpenSpec: the spec says nothing about the harness, and a proposal for it is impossible under the process's own rules. The harness is edited directly, in its own worktree; decisions are recorded in `knowledge/decisions.md`.

### A worktree per session

A session that edits files works in a separate git worktree, not in the shared directory: the index and the working tree are shared per directory, and one session's commit carries away another's files. One worktree per change or per harness PR.

Worktrees are created by Orca. The main session is a curator in the main directory and edits no files there; a batch of edits goes to a child worktree with a supervised executor: `orca worktree create` → `orca orchestration worker-start` → `check` → acceptance → `orca worktree rm`. One task or batch of edits — one worktree, a child of the main directory: `orca worktree create --parent-worktree path:<main directory>`, without `--no-parent`. That way Orca shows whose work it is and which task it belongs to. Commands and pitfalls — the `orca-cli` and `orchestration` skills (Orca installs them itself). A worktree is not left without a running executor, and the empty terminal that `orca worktree create` opens by itself is closed after the executor starts. The directory the session started from is not removed: hooks resolve from it.

Single-threaded work without other sessions may use the shared directory, but before a commit still run `git status` and stage files by name. The session start hook reports where the session works and which worktrees are taken; in the shared directory bulk staging (`git add -A`, `git add .`, `git commit -a`) is rejected by a hook.

### Don't edit generated files by hand

Only with the corresponding command:

- `target/**`, `Cargo.lock` — cargo;
- `.claude/commands/opsx/**`, `.claude/skills/openspec-*/**`, `.pi/prompts/opsx-*`, `.pi/skills/openspec-*/**`, `.agents/skills/openspec-*/**` — `openspec update`;
- blocks between `<!-- … -->` markers in `knowledge/` documents — the `knowledge/` scripts;
- `.rootstock.toml` and managed template files — `rootstock update`;
- project generated files — per the "Sources of truth" table in the root `AGENTS.md`.

### Editing files

Edit files with the client's edit tools: Write and Edit in Claude Code, `write` and `edit` in Pi, or `apply_patch` in Codex. These edits trigger formatting, checks for the touched package, and a procedure reminder when a contract is touched. Use the shell for running commands.

## Commands

Package managers — `cargo` for Rust. No wrappers or Makefile.

```
# Rust
cargo check
cargo test
cargo clippy --all-targets --all-features -- -D warnings
cargo fmt --check
cargo run
```

Project commands (database, running services, generation) — in the root `AGENTS.md`.

## Order of work

Implement strictly stage by stage from the spec's stages section. Don't start the next stage until the current one passes its acceptance criterion. After each stage — a short report: what was done, what deviated from the spec and why, what could not be checked without live data.

## Commits

- A commit per completed step, not per file. The prefix is a package or area (`<package>:`, `chore:`, `docs:`); the list of prefixes is project-specific.
- A PR title names the change or the harness task.
- The commit of a completed stage carries a short report in its body.
- Checks: on every edit — agent hooks; before a PR — a full run in the main session (`change-finish`); after — green CI. Pre-push runs nothing heavy.
- `.env`, secrets and local configs are never committed. Connection strings and tokens live in the environment or a local config, but not in defaults.

## Code rules

- **Identifiers are English words.** Names of functions, variables, fields, types, tests and constants are written in English, without transliteration.
- All thresholds, zones, windows and timeouts — only via config with defaults from the spec. No magic numbers in code.
- All timestamps are UTC.
- Rust stable, edition 2024.
- In Rust: `tracing` for logs, `println!` is forbidden everywhere; `unwrap()`, `expect()`, `unsafe` outside tests are forbidden; blocking calls in an async context are forbidden.

## What not to do

- Don't invent requirements, metrics or table fields — if something is missing, describe it in the report and ask.
- Don't introduce an ORM.
- Don't use `unsafe`, `unwrap()` outside tests, or blocking calls in an async context.
- Don't edit managed template files in the project: your own — into extension points (below), shared — a PR to harness (the `harness-upstream` skill).
- Don't write long explanations in chat — code, tests and a short report.

## Repository harness

- **Skills:** `change-finish`, `contract-change`, `harness-upstream`, `knowledge-note`, `stage-acceptance`, `stage-grooming`. Which one when — in their descriptions. The project's own skills sit next to them under their own names.
- **Subagents:** executor `rust-developer`; reviewers `rust-reviewer`, `spec-auditor`. Executors read the skills named in their prompts. The prompts are shared across Claude Code (`.claude/agents/`), Pi (`.pi/agents/`, through the `pi-subagents` extension), and Codex (`.codex/agents/`). Codex inherits the parent session's model.
- **Agent addenda:** `.harness/local/agents/<name>.md` — domain, invariants, project skills for the agent `<name>`; the agent reads it before starting work, and on conflict the addendum wins over the shared prompt. The `.harness/local/` directory belongs to the project; what lives there — its README.
- **Skill addenda:** `.harness/local/skills/<skill>.md` — the project's own additions to a template skill: who merges PRs, who is who in the tracker, how to check a rollout, and other rules the skill calls a "project rule". The skill reads it as its first step, and on conflict the addendum wins over the shared text.
- **Hooks** live in `.harness/hooks/` and are called by one dispatcher per event from `.claude/settings.json`, the `.pi/extensions/harness.ts` extension, and `.codex/hooks.json`. They run formatting and package checks on edit, guard the shared directory on shell commands, and report worktree status at startup and exit. An executor (`*-developer`) with failing rustfmt or clippy checks gets the error output and one chance to retry before handing in its report. Put project hooks in `.harness/local/hooks/<slot>/`. The contract is in `.harness/hooks/README.md`.
- **Git hooks (lefthook):** pre-commit — formatting staged files, secrets, moving closed questions to the archive and document tables of contents via the project's `knowledge/index.sh`, if it exists; pre-push — only rare invariant checks. Full checks — in CI.
- **OpenSpec:** propose/apply/archive, configuration in `openspec/config.yaml`, state — `openspec list`, `openspec status`.

### Head and hands split

Decisions are made in the main session, executors do the handwork. A task for an executor is self-contained: the `tasks.md` item, the files it touches, the decision made, and the done criterion. "Figure it out and do it" is not handed to an executor — that is a decision, and it is yours.

The main session keeps: reading and interpreting the spec, choosing between implementation options, any contract change (`contract-change`), editing `openspec/changes/<name>/`, commits and stage acceptance.

An executor that returned a question instead of code did the right thing. Answer the question and rerun it — don't rephrase the task so that it "just picks itself".
