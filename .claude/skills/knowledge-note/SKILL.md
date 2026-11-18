---
name: knowledge-note
description: Use when recording something learned — a decision and its reason, a defect review found, a question the spec does not answer, or a quirk of an external API. Decides which of the three documentation places it belongs to and the shape of the entry.
---

# Recording in knowledge

First check `.harness/local/skills/knowledge-note.md`: if the file exists, read it before starting work. It is the project addendum — the project's own kinds of entries and their shape; where it disagrees with the text below, the addendum wins.

The three documentation places don't mix: the spec (`docs/design.md`) holds requirements (owned by the owner; we read it and ask), `openspec/specs/` holds implemented behavior, `knowledge/` holds why it is so and what we found out. A wrong address costs more than it seems: a requirement written into knowledge is approved by nobody, and behavior described there will silently drift from `openspec/specs/`.

## Where

| The entry starts with… | Where |
|---|---|
| "The system must…" | This is a requirement. Either it is already in the spec, or it has to be asked. Nowhere here. |
| "Given such-and-such input, the system does…" | Behavior. `openspec/specs/`, via OpenSpec. |
| "We chose X over Y because…" | `knowledge/decisions.md` |
| "Review found that…" / "Measurement showed that…" | `knowledge/lessons.md` |
| "Unclear how to compute…" / "The spec is silent, the code decided itself…" | `knowledge/open-questions.md` — and in the stage report |
| "The external API returns at most N records, so…" | a topic file in `knowledge/` (list — `knowledge/README.md`) |

Decisions and lessons differ not in importance but in origin: a decision is a choice, a lesson is a defect that was found plus the conclusion from it. Mixing them kills the journal's scannability, so the separation is kept literally.

## Shape

**Decision** (`knowledge/decisions.md`) — append-only, a new entry at the end. Date, decision, reason, consequence:

```markdown
## YYYY-MM-DD · Short statement of the decision

**Decision.** What exactly was decided, in one sentence.

**Reason.** Why this way and not another. The rejected option is named.

**Consequence.** What now holds in the code and what breaks if it is violated.
```

A decision is not rewritten. Changed your mind — a new entry with a link to the one being revoked.

**Lesson** (`knowledge/lessons.md`) — the same shape. The reason says how the defect was found: review, measurement, failure, live data. A lesson without the means of detection is useless: it won't teach anyone to catch the next one like it.

**Open question** (`knowledge/open-questions.md`) — a question as the heading, the gist, and a mandatory "Due: …" line. A question without a deadline becomes eternal.

The deadline must name a stage number — `Due: stage 4.` — or a change with the stage number in its name: ``Due: `stage1-<name>`.``. The stage number comes from the stage table `knowledge/stages.md`. There are no other forms: the deadline is read by `knowledge/due.sh`, and a prose "by the stage where it's needed" can't be checked, so it counts as missing. The condition stays in the question text; the deadline becomes the stage at which the condition is expected. After that, the deadline is presented at the entry to the stage — the `stage-grooming` skill.

Closing a question — a struck-through heading with a link to the decision or spec paragraph:

```markdown
## ~~Question title~~ — closed YYYY-MM-DD
```

There is no need to move a closed section to the archive by hand: the `knowledge/` scripts do it at pre-commit.

## Honesty rule

An entry here is a statement about reality. Anything unverified is marked with the word "assumption" and stays so until verified on live data or in documentation. The mark can be removed only together with a link to the verification.

Numbers are given with the measurement conditions: how long, how many events, on what data, in what mode. A number without conditions is not reproducible and therefore useless.

## What not to do

- Don't edit blocks between `<!-- … -->` markers in documents: the `knowledge/` scripts build them.
- Don't rewrite an old entry to fit a new understanding. The journal is valuable because it shows what was thought at the time.
- Don't create an entry "just in case": what is visible from the code, a commit, or the spec is not needed here.
