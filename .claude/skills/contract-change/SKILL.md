---
name: contract-change
description: Use when changing anything listed in the "Sources of truth" table of the project's AGENTS.md — a contract between packages or processes, a database schema, a serialized format, config defaults, or anything generated from them. The spec-first, regenerate-then-verify-consumers procedure.
---

# Changing a contract

First check `.harness/local/skills/contract-change.md`: if the file exists, read it before starting work. It is the project addendum — the project's own regeneration and consumer verification steps; where it disagrees with the text below, the addendum wins.

A contract is anything someone else depends on: a protocol between processes, a database schema, a format of stored data, configuration defaults, generated code. Changing any of them is a procedure, not a file edit: each has its own consumers and its own regeneration.

## Which contracts the project has

The list is the "Sources of truth" table of the root `AGENTS.md`: what, where, who changes it and how. The skill keeps no list of its own. No row, but the file is clearly a contract (another package, the client, the database, or stored data depend on it) — first ask whether to add a row, and only then edit.

For a table row, find out three things before the edit: who the consumers are, what regenerates the derived files, what verifies the consumers. Not recorded — find out from the code and add it to the table in the same edit.

## Order

**1. Spec first.** The needed field, metric, state, or value isn't in the spec (`docs/design.md`)? Don't add it. Describe what is missing and ask. The spec is the source of truth; code follows it, not the other way round. Configuration defaults are checked against the spec's values.

**2. Change the contract as a whole.** A new field is added so that an old consumer doesn't break (a default value on read). A breaking change bumps the contract version, if there is one, and is fixed in all consumers in the same commit.

**3. Regenerate.** Derived files — only by the command from the table, never by hand. A generated file edited by hand is overwritten by the very first regeneration, and until then it lies.

**4. Verify the consumers.** Build and typecheck of all consumers — mandatory, for any contract:

```
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test
```

A change to the format of stored data is additionally checked by reading existing records and fixtures.

**5. Tests.** For a database schema — migrations from scratch and a write-read round trip.

**6. Commit in one piece.** Contract, consumers, generated files, and tests — in one commit. An intermediate state where generated files lag behind the source breaks the build for everyone.

## Forbidden shortcuts

- Editing an already applied migration instead of adding a new one. The schema changes only by adding a migration.
- Editing generated files by hand to "quickly fix the build".
- Hardcoding a threshold or value "temporarily" instead of a configuration key with a link to the spec paragraph.
- Renaming a field in code and forgetting that it goes into serialization: wire names are set explicitly, not derived from names in code.
- Changing the format of stored data without checking old records. If compatibility can't be kept, the records are recreated, and this is noted in the stage report.
