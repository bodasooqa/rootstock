---
name: stage-acceptance
description: Use when all changes of a stage are archived and the stage is about to be declared done — the acceptance procedure before the stage-closing commit and before any change of the next stage may start.
---

# Stage acceptance

First check `.harness/local/skills/stage-acceptance.md`: if the file exists, read it before starting work. It is the project addendum — the project's own acceptance checks; where it disagrees with the text below, the addendum wins.

The most expensive mistake in a project is declaring a stage closed when it isn't: the next stage is built on an unverified foundation, and the divergence surfaces several changes later, when fixing it is expensive. That is why acceptance is a procedure, not a feeling.

Acceptance is the main session's work. It is not delegated to a subagent: it is a decision.

## When

All changes of the stage are archived. Check by fact, not by memory:

```
openspec list                     # no active changes of the stage left
ls openspec/changes/archive       # all stage<N>-* names are in place
```

A change of the next stage doesn't start until this procedure is completed.

## Criterion

The stages section of the spec (`docs/design.md`) names the content of each stage and its acceptance criterion. Where the criterion isn't written explicitly, the criterion is the capabilities listed in the item, working on live data.

Write out the criterion verbatim and answer for each of its parts separately: verified, partially verified, not verified. "Seems to work" means "not verified".

Some criteria can be verified only on live data. That is no reason to skip them and no reason to lie: what is unverified goes into the report as a separate list rather than dissolving into "stage closed".

## Order

**1. Green checks.** In full, not per affected package:

```
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test
```

**2. Reviewers.** `spec-auditor` — mandatory; it checks the implementation against the spec paragraph by paragraph. `rust-reviewer` — on the Rust part of the stage. Their findings are attached to the report; a divergence is either fixed or named as a deviation with a reason.

**3. Live check.** A stage that shows or writes something is checked on live data, not only on synthetic data. Whatever couldn't be checked live goes into the report.

**4. Knowledge.** Decisions made during the stage go to `knowledge/decisions.md`; defects found by review and the conclusions from them go to `knowledge/lessons.md`; everything where the spec is silent and the code made the decision goes to `knowledge/open-questions.md`. The procedure is the `knowledge-note` skill.

**5. Stage table and README.** In the stage status table `knowledge/stages.md`, the stage's row gets `closed` and the next one `in progress` (states are exactly `not started`, `in progress`, `closed`); edited in this same commit. This is what declares the closing: `knowledge/due.sh` reads the table. Along the way, the root `README.md` is checked — repository map, quick start, commands: a stage usually brings a new package or a new command. An outdated README is the first to lie and the last to be noticed.

**6. Report.** Short, three parts: what was done; what deviated from the spec and why; what couldn't be checked without live data. The report goes into the body of the stage-closing commit.

**7. Commit.** A separate stage-closing commit with the report in its body. Files are named one by one.

**8. Grooming of the next stage.** Acceptance only creates questions; they are presented at the entry to the next stage — the `stage-grooming` skill. After the closing commit, `knowledge/due.sh --overdue` shows what came due together with the stage closing: questions addressed to it and its changes became overdue exactly now.

## What not to do

- Don't declare a stage closed with part of the criterion unmet, postponing it "to the next stage". The criterion is either met or the stage is open.
- Don't add to the criterion from your head. If the stages section sounds vague, that is a question for the owner, not a place for a guess.
- Don't replace a live check with a synthetic test where the criterion talks about behavior on live data.
- Don't start the propose of the next stage before the closing commit of the current one and before grooming (`stage-grooming`).
