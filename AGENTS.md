# AGENTS.md — rootstock

What the project does, in one paragraph. The spec — `docs/design.md` — is the source of truth about behavior; this file holds the working rules.

Shared process rules come from the [harness](https://github.com/bodasooqa/harness) template, in the managed file `.harness/process.md`: read it in full before starting work. Claude Code gets it via an import from `CLAUDE.md`, Pi via the `.pi/extensions/harness.ts` shim, other agents read the file themselves. This file belongs to the project: the template no longer touches it. Project rules are written here, not in `CLAUDE.md`: that file only pulls in this one and the process rules for Claude Code.

## Project stack

Languages, key libraries, and rules for using them that the shared rules don't cover.

## Package boundaries

Map of packages and the dependency graph between them; invariants this graph upholds.

Read the local `AGENTS.md` of every package you touch, even when your session started at the project root. The neighboring `CLAUDE.md` imports those rules for Claude Code.

## Sources of truth

What in the repository is a contract or generated, and how it changes. Changing any row of this table is a procedure, not a file edit: the `contract-change` skill.

| What | Where | Who changes it |
|---|---|---|
| Domain model, requirements, defaults | `docs/design.md`, revision journal `docs/design-revisions.md` | revisions per the owner's decisions |
| OpenSpec commands and skills | `.claude/commands/opsx/`, `.claude/skills/openspec-*/`, `.pi/prompts/opsx-*`, `.pi/skills/openspec-*/`, `.agents/skills/openspec-*/` | **generated**, only `openspec update` |
| Document tables of contents and summary blocks | blocks between `<!-- … -->` markers in `knowledge/` | **generated**, only the `knowledge/` scripts |

## Definitions

Domain terms that are easy to confuse — briefly, with a link to the spec section.
