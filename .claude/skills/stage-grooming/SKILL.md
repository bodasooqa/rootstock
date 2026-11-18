---
name: stage-grooming
description: Use when a stage is about to start — before the first change of the stage, and whenever knowledge/due.sh --overdue turns red. Reviews open questions whose deadline has come.
---

# Grooming before a stage

First check `.harness/local/skills/stage-grooming.md`: if the file exists, read it before starting work. It is the project addendum — the project's own grooming rules; where it disagrees with the text below, the addendum wins.

Every open question carries a "Due: …" line — an IOU: we'll deal with it by then. If nobody presents the IOU, after a couple of stages most questions turn out overdue — addressed to stages closed weeks earlier. Stage acceptance only creates questions; grooming is the entry, symmetrical to `stage-acceptance`.

The stakes are the same as for acceptance: a question where the spec was silent and the code made the decision is cheapest to fix before the next stage is built on that guess.

Grooming is the main session's work. It is not delegated to a subagent: closing a question is a decision, and some questions are addressed to the spec owner altogether.

## When

Before the first change of a new stage — that is, right after the commit closing the previous one (the "closed" row in the stage table `knowledge/stages.md`). A change of the next stage doesn't start until grooming is done.

Also — whenever `knowledge/due.sh --overdue` turns red. A red check means exactly one thing: the IOU has been presented and there is no answer to it.

## Order

**1. Selection.** The list is printed by the script, not by memory:

```
knowledge/due.sh <stage number>    # what is addressed to the stage plus all overdue
knowledge/due.sh --overdue         # overdue only
```

Everything it printed is handled — both what has come due and what is overdue. Questions whose stage is still ahead are left alone.

**2. Three outcomes, no fourth.** For each entry of `knowledge/open-questions.md`:

- **Closed.** There is an answer — in `knowledge/decisions.md`, in a spec paragraph, or in the owner's answer. The heading is struck through with a link: `## ~~Question title~~ — closed YYYY-MM-DD`.
- **Postponed.** There is no answer yet and there can't be — for example, live data or a part of the system that doesn't exist yet is needed. The "Due:" line is rewritten to the new stage, and a trace appears below it: `Postponed YYYY-MM-DD from stage N: <reason>`. The reason is mandatory — a postponement without a reason is exactly the formality that makes the gate pointless.
- **Obsolete.** The question fell away: the code was rewritten, the requirement went away, the metric isn't being added. It is closed like a closed one, with a link to what cancelled it.

**3. A question can outlive its answer.** Before taking a question to the owner, check its text against the current version of the spec and against the decisions. A question quoting an old version of the spec may ask for a decision that a later revision has already made. An entry about a missing answer doesn't update itself when the answer appears elsewhere.

**4. The deadline must name a stage.** There are two machine forms: a stage number (`Due: stage 4.`) and a change with the stage number in its name (`stage1-<name>`). A prose deadline ("the stage where it's needed") counts as missing: there is nothing to check it with. The condition stays in the question text; the deadline becomes the stage at which the condition is expected.

**5. A third postponement is not a postponement.** If the section already has two "Postponed …" lines, the question can't be postponed. It is either closed or goes to the spec owner as a separate question: three postponements in a row mean the answer depends not on a stage but on a decision nobody has made.

**6. Checking the trace.** A grooming after which nothing changed did not happen:

```
git diff --stat knowledge/        # not empty
knowledge/due.sh --overdue        # green
```

If anything overdue remains, it remains deliberately and is named in the report, not because nobody got around to it.

**7. Commit.** A separate documentation commit listing: what was closed, what was postponed and why, what went to the owner. Files are named one by one.

## What not to do

- Don't close a question with your own decision where the spec is silent and the answer changes product behavior. Such a question goes to the owner — that is exactly what it is for.
- Don't postpone "to make the list green". A green list bought with postponements lies exactly like an overdue one.
- Don't create new questions from your head during grooming. A question is created where it is found: in apply, in review, in acceptance — the `knowledge-note` skill.
- Don't start the first change of a stage until `due.sh --overdue` is green or the overdue items are named in the report.
