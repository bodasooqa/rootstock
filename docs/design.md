# rootstock: design

Date: 2026-10-29. Written by Claude from a conversation with the owner (bodasooqa). "The owner decided" marks the owner's decisions; "my call" marks decisions I made that the owner may overturn.

Revisions 1.1 to 1.4 (2026-10-30) are merged into this text; their changes are marked "(1.1)" to "(1.4)", and the reasons are in `docs/design-revisions.md`.

## Intent

rootstock is a command-line tool, written in Rust and shipped as a single binary, that creates a project from a template **and later updates that project to newer versions of the template without losing the project's own edits.**

In a rootstock-managed project the template is the rootstock and the project is the scion grafted onto it: the project grows its own branches, and improvements to the shared root keep reaching it.

### Why

The owner maintains [harness](https://github.com/bodasooqa/harness), a shared template of tooling and agent process (Claude Code and Pi agents, hooks, CI, lefthook, knowledge scripts) that several projects consume. Its whole point is that **improvements to the template reach every project**: a scheduled CI job opens a pull request in each project with the new template version, and that PR merges itself when there are no conflicts and CI is green.

Today this runs on [Copier](https://copier.readthedocs.io/) 9.19, which is written in Python. The owner's stack is Rust (and TypeScript for the frontend), and he does not want a Python dependency in CI and on every machine just for this. He looked for a Rust tool and found none that covers the need, so the plan is to build one and make it a public open-source project, since others are likely in the same position.

### Prior art (checked 2026-10-29)

| Tool | Language | Templates | Updating an existing project |
|---|---|---|---|
| Copier | Python | Jinja2 | Yes, three-way: renders the old and new template versions with the saved answers and merges the difference into the project. This is what rootstock reproduces. |
| cargo-generate | Rust | Liquid | No, creation only. |
| kickstart | Rust | Tera | No. |
| ffizer | Rust | Handlebars | A `merge` mode that opens the user's configured git mergetool per file, with the raw template file (not the previous render) as the base. Interactive, not usable in CI. |
| Baker | Rust | minijinja | `baker update` exists but is two-way: every file that differs from the new render gets conflict markers around the whole file, and non-template files are overwritten silently (`src/cli/processor.rs:157` at `b4d2a2c`). No way to choose a template version (it always takes the default branch). No `trim_blocks`/`lstrip_blocks`/`keep_trailing_newline`. 62 stars. |
| rusticate | Rust | none (string renaming + comment-marked blocks) | `cargo rusticate upgrade` does the right three-way merge (`git merge-file`), but it is a generator for one specific backend template, not a general tool. |

The gap: **no Rust tool updates a project correctly** — with the previous render as the merge base, so that only genuinely overlapping edits become conflicts. That is the reason anyone picks Copier, and the reason rootstock exists.

The owner decided to build a new tool rather than extend Baker: the parts that matter (update semantics, file classes, versioned templates) would replace Baker's update path wholesale and change its behaviour for existing users.

## Users

1. **harness** is the first user, and through it the owner's projects (screener first).
2. Then anyone who maintains a template meant to live, not to be stamped out once: in-house company templates, agent harnesses like harness and rusticate, shared CI and lint setups. *(My guess at the audience; the owner agreed.)*

## Goals

1. `rootstock new` creates a project from a versioned template, asking the template's questions (or taking answers non-interactively).
2. `rootstock update` brings an existing project to a newer template version with a three-way merge. Edits the project made are kept; edits that overlap with the template's change become standard conflict markers; nothing is lost silently.
3. The template declares explicitly which files the template keeps managing (**managed**) and which are created once and then belong to the project (**starter**).
4. Runs unattended in CI: no prompts when told not to, stable exit codes, a machine-readable list of conflicts.
5. `rootstock import-copier` converts a Copier template and a Copier-generated project to rootstock, once.
6. harness moves off Copier onto rootstock, and its daily update PR keeps working as it does now.

## Non-goals

- **Running Copier templates unconverted.** The owner decided on an own format plus a one-time converter. Reading `copier.yml` as-is would promise compatibility that cannot be kept: Copier templates often load Python Jinja extensions, minijinja differs from Jinja2 in small ways (each one a spurious diff on update), and we would have to reproduce the very Copier behaviours we are moving away from.
- **Talking to GitHub or any other host.** rootstock does not open pull requests, set labels or know about CI. harness keeps its own workflow and calls `rootstock update` from it. *(My call.)*
- **Migration scripts between major template versions** (Copier's `_migrations`). harness handles majors with manual steps in its CHANGELOG. Hooks that run on update cover the simple cases. Can be added later. *(My call.)*
- **Rename detection.** A renamed template file is a delete plus an add (see the update rules). *(My call.)* A rename that only changes the case of a name is treated as one file only where the file system folds case and the project still has that file; the project's edits then move to the new spelling by the usual merge. Anywhere else it is a delete plus an add, so a starter file the project deleted can come back under its new spelling. *(Owner decided, 1.1.)*
- **Python, or any runtime besides the binary and `git`.**

## Anti-goals

Taken from the owner's own words about harness: **conflicts on update are a normal part of the work, not a failure. The failure is an update that silently overwrites what the project changed, or an improvement that silently fails to reach a project.** Every update rule below is judged against this.

## Constraints

- Rust stable, edition 2024. One binary. Linux, macOS and Windows builds. On Windows, rootstock behaves as on Unix: the executable bit and symlinks come from the template's git tree, and hooks run in a POSIX shell (see Hooks and update rule 7) *(owner decided, 1.4)*.
- Templating: Jinja syntax via [minijinja](https://github.com/mitsuhiko/minijinja), so harness file contents carry over with little or no change.
- Templates come from git repositories; versions are git tags.
- English everywhere public: README, docs, CLI messages, code comments, **commit messages**. **No `Co-Authored-By: Claude` line in commits.** *(Owner decided.)* The repository's own agent tooling (`CLAUDE.md`, `knowledge/`), generated from harness, stays in Russian like harness itself.
- License: MIT OR Apache-2.0. *(My call: the Rust convention.)*

## Concepts

- **Template** — a git repository (or a subdirectory of one) containing a `rootstock.toml` and the files to render. The directory that holds `rootstock.toml` is the **template directory**. *(1.1)*
- **Version** — a git tag of the template repository, semver `vX.Y.Z`. A project records both the tag and the exact commit.
- **Answers** — the values the user gave to the template's questions.
- **Record** — a file at the project root, `.rootstock.toml`, holding the template source (with its subdirectory, if any *(1.1)*), the version (tag and commit), the answers and the inventory. It is committed with the project and is the only link between project and template. *(File name my call.)*
- **Inventory** — the list of paths rootstock emitted into the project, each with its class. It lets rootstock tell "the project deleted this file" from "this file was never emitted".
- **File classes:**
  - **managed** (default) — rootstock keeps it in sync with the template on every update, merging with the project's edits.
  - **starter** — emitted once by `new` (or the first time the template introduces it), then owned by the project. Updates never touch it, and a starter file the project deleted is never recreated.
  - **excluded** — never emitted (template sources, partials, docs about the template itself).

## Template format

Template configuration lives in `rootstock.toml` in the template directory *(1.1)*. *(TOML my call: native to the Rust ecosystem and free of YAML's implicit typing. Copier and Baker use YAML.)*

It declares:

- **Questions**, in order. Types: `bool`, `str`, `int`, `choice` (single), `multichoice`, `yaml` (structured value). Each may have `help`, `default` (may be a Jinja expression over earlier answers), `when` (Jinja expression; a question whose `when` is false is not asked, takes its default, and is not written to the record — same as Copier) and `validator` (Jinja expression that renders to an error message, empty when valid). As in Copier, `default` and `validator` are templates (`{{ … }}`) rendered over the earlier answers, while `when` is a bare expression. *(Owner decided, 1.1.)*
- **Template root** — subdirectory of the template directory holding the files (harness uses `template/`). When the template root is the template directory itself, a `.rootstock.toml` at its top is the repository's own record as a rootstock project (a template that is itself a project) and is never rendered, like `rootstock.toml` *(my call, 1.3)*.
- **Templated files** — suffix `.jinja` by default (configurable): such files are rendered and the suffix dropped. Everything else is copied byte for byte. File and directory names are always rendered; a name that renders empty is not emitted. That is how `{% if rust %}Cargo.toml{% endif %}` works, as in Copier.
- **Jinja environment options** — `trim_blocks`, `lstrip_blocks`, `keep_trailing_newline`, all configurable. `keep_trailing_newline` defaults to true.
- **Class rules** — gitignore-style glob lists for `starter` and `exclude`, evaluated against the rendered path. A pattern may contain Jinja over the answers (harness's `{{ spec_path }}/**`). Patterns render with the answers on every step, including the base render during update. This avoids the Copier bug where `_skip_if_exists` rendered without answers turned `{{ spec_path }}/**` into `/**`.
- **Includes** — `{% include %}` and `{% import %}` resolve against the template directory, not the template root, as in Copier: harness's `template/_partials/…` paths carry over unchanged. The include base is not configurable. *(Owner decided, 1.1.)* A configurable list of partial directories is excluded from output.
- **Paths and links** *(owner decided, 1.1)*. A rendered path containing `\`, `:` or a NUL byte is refused. A template symlink is refused at render when its path or target is not ASCII, its target is absolute, contains `~`, or points outside the project.
- **Hooks** — shell commands with a `when` of `new`, `update` or both, run in the project directory after files are written. Hooks run only with `--trust` or after an interactive confirmation, since they execute arbitrary commands. A hook runs as `sh -c <command>`. On Windows, `sh` is the one shipped with Git for Windows, found next to `git`; without it a command with hooks to run is an error before anything is written *(owner decided, 1.4)*.
- **Minimum rootstock version.**

Filters and tests: minijinja's built-ins, plus `regex_search` with the semantics of the jinja2-ansible-filters version Copier ships (harness's `project_name` validator uses it). `to_nice_yaml` is not needed: in harness it only renders the Copier answers file, which the converter drops (see import-copier). *(My call; it can be added later for other templates.)*

Python methods on values, as Jinja2 allows them *(owner decided, 1.2)*: the common methods of strings (`startswith`, `endswith`, `lower`, `upper`, `strip`, `split`, `replace`, …), mappings (`items`, `keys`, `values`, `get`) and lists, with Python semantics. `list.append(x)` changes the list in place, as in Python: harness builds a list with `{% set names = [] %}` and `{% set _ = names.append(name) %}`. Every harness tag uses `append` and `endswith`, and old tags are rendered as they are. A method outside this set is a render error, as before. *(The exact set: my call, the builder's.)*

## Commands

- `rootstock new <template> <dir> [--subdir path] [--version vX.Y.Z] [--data k=v]... [--data-file f] [--defaults] [--trust] [--git] [--format text|json]` — creates the project at the given version (default: latest semver tag), writes the record. Does not run `git init` unless the directory is empty and `--git` is passed *(my call)*; `--git` with a non-empty directory is an error *(owner decided, 1.1)*. A non-empty directory is otherwise allowed: a file already there is treated as "— / managed / present" and differing content is an `added-by-both` conflict *(owner decided, 1.1)*. `--format` applies to the conflict list *(1.1)*.
- `--subdir` names the template directory inside the template repository, for every kind of source; the record keeps it next to the source, so `update` and `status` use it without the flag. A local path may also point straight into the subdirectory. *(Owner decided, 1.1.)*
- `rootstock update [--to vX.Y.Z] [--data k=v]... [--data-file f] [--defaults] [--trust] [--format text|json]` — updates the project in the current directory. `--data` (and `--data-file` *(1.1)*) changes an answer: the base renders with the old answers, the new side with the new ones. This works at the current version too: the update then renders the same version with both sets of answers *(my call, 1.3)*.
- `rootstock status [--format text|json]` — current version, latest available, whether the step is a major bump. Replaces harness's `next-version.sh`.
- `rootstock check` — fails if any managed file contains unresolved conflict markers or the record is inconsistent with the tree. For CI. Inconsistent means: the record does not parse, an inventory path is not a safe relative path, or the recorded version is neither a `vX.Y.Z` tag nor `HEAD`. It needs no template checkout. A missing managed file is not an error *(owner decided, 1.3)*.
- `rootstock import-copier` — see below.

Template sources: `https://…`, `git@…:…`, `gh:owner/repo` shorthand, and a local path. For a local path, the version is a tag in that repository; `--version HEAD` renders the working tree, for template development, including untracked and ignored files *(owner decided, 1.1)*.

## How update works

1. **Clean tree required.** If the working tree has uncommitted changes, refuse. Every update can then be reviewed and undone with git. *(My call, after rusticate.)* Likewise, if a managed file still holds conflict markers from an earlier update, refuse with exit 1: merging over unresolved markers would bury them. *(Owner decided, 1.1.)*
2. **Resolve versions.** Read the record. Target = `--to` or the latest semver tag. Already at the target and no answer changed: exit 0, nothing to do *(1.3)*. `--to` may name an older tag than the recorded one *(owner decided, 1.1)*.
3. **Ask new questions.** Questions added by the new version take `--data`, then their default under `--defaults`. A new question with no default and no answer is an error when there is no terminal. Without `--defaults` and without a terminal, a new question is an error even when it has a default *(owner decided, 1.1)*.
4. **Render twice into temporary directories:** the *base* (recorded commit, recorded answers) and the *target* (target commit, current answers). Class rules are evaluated per side with that side's answers.
5. **Apply per path**, with project = the file on disk:

   | Base | Target | Project | Result |
   |---|---|---|---|
   | — | managed | absent | create |
   | — | managed | present | three-way merge with an empty base: identical → nothing; different → conflict |
   | — | starter | absent, never in inventory | create |
   | — | starter | absent, in inventory | nothing (the project deleted it) |
   | any | starter | present | nothing |
   | managed | managed | = base | write target |
   | managed | managed | ≠ base | three-way merge; overlaps → conflict markers |
   | managed | managed | absent | base = target: nothing. base ≠ target: conflict ("deleted by project, changed by template"), the file stays absent. *(My call: an improvement the project would miss is never silent; a project that wants the file gone for good should get it reclassified as starter in the template. Nothing extra is remembered in the record.)* |
   | managed | — | = base | delete |
   | managed | — | ≠ base | keep, report as conflict ("removed by template, edited by project") |
   | managed | — | absent | nothing |
   | managed | starter | present | nothing; from now on it belongs to the project |
   | managed | starter | absent | nothing *(owner decided, 1.1)* |
   | starter | managed | any | three-way merge against the base render; the template takes the file back. Absent in the project: create *(owner decided, 1.1)* |
   | any | excluded or not emitted | — | as "managed → —" if it was managed, nothing if starter |

6. **Binary files** (managed): if the project's copy equals the base, replace it; otherwise keep the project's copy, write the target next to it as `<name>.rootstock-new`, and report a conflict. A binary file the template did not change (base = target) is left as the project has it, with no side file and no conflict; otherwise every update would report it again. If the template itself emits `<name>.rootstock-new`, no side file is written and the conflict is still reported. A file is binary when its first 8000 bytes contain a NUL or it is not valid UTF-8. *(Owner decided, 1.1.)*
7. **Permissions and links.** The executable bit follows the template file (rendered or copied). Symlinks are reproduced as symlinks, never followed. Both are read from the template's git tree (mode `100755`, `120000`), not from the checkout on disk, so they do not depend on the file system; untracked files of a `HEAD` render take them from disk. Where the file system has no executable bit (Windows), rootstock sets it in the project's git index (`git update-index --chmod=+x`) when the project is a git repository, and reads the project's executable bit from that index. A symlink the operating system refuses to create (Windows without developer mode) is an error before anything is written, naming how to allow it *(owner decided, 1.4)*. Like a binary file, a symlink the template did not change is left as the project has it. Links that differ in target, or a file on one side and a link on the other, that cannot be resolved by the table are a `symlink` conflict, and the project's entry is kept. *(Owner decided, 1.1.)*
8. **Bytes in, bytes out.** No line-ending conversion. The merge works on lines but preserves the file's line endings and final newline.
9. **Conflict markers** are git-style (`<<<<<<< project`, `=======`, `>>>>>>> template vX.Y.Z`), so `git diff --check`, editors and harness's `conflict-markers.sh` see them.
10. **Run update hooks**, then write the new record: new version, current answers, new inventory. Hooks that need trust, without `--trust` and without a terminal to confirm, stop the whole command before anything is written *(owner decided, 1.1)*.
11. **Exit codes:** `0` clean, `1` error (nothing written, or a clear statement of what was), `2` done with conflicts. Every outcome the table or rule 6 calls a conflict — markers in a file, a `.rootstock-new` next to a binary, "removed by template, edited by project", "deleted by project, changed by template" — gives exit 2. There is no "reported but clean" state: CI reads only the exit code, so anything that needs a human must be exit 2. The conflicting paths with their kind are printed one per line on stdout, or as JSON with `--format json`. The kinds are `markers`, `binary`, `symlink`, `removed-by-template`, `deleted-by-project`, `added-by-both` and `blocked` *(1.1)*.
12. **Blocked paths** *(owner decided, 1.1)*. When a path the template emits is taken by something the project keeps — a file where the template needs a directory, or a directory with files or subdirectories where it needs a file — the template's entry is dropped, kept out of the inventory and reported as a `blocked` conflict. Nothing the project keeps is removed to make room.

New and update share one renderer and one apply step; `new` is an update from an empty base into an empty project.

## import-copier

Converts once; afterwards Copier is not needed.

- **Template side** (run in a Copier template repository): reads `copier.yml` and writes `rootstock.toml`. Supported subset — exactly what harness uses: question types `bool`/`str`/`yaml`, `choices`, `when`, `validator`, `default`, `help`; `_subdirectory`, `_templates_suffix`, `_envops`, `_exclude`, `_skip_if_exists`, `_tasks`, `_answers_file`, `_min_copier_version`. Mapping: `_skip_if_exists` → `starter`; `_exclude` entries of the form `{% if _copier_operation == 'update' %}…{% endif %}` are dropped as redundant once those paths are starter; other `_exclude` → `exclude`; `_tasks` → hooks with the matching `when`. The template file that renders the Copier answers file (`{{ _copier_conf.answers_file }}.jinja`, content `{{ _copier_answers|to_nice_yaml }}` in harness) is dropped: the record replaces it. Anything else outside the subset (`_jinja_extensions`, `_migrations`, unsupported question types, other uses of `_copier_conf` or `_copier_answers`) stops the import with a list of what is unsupported. It never converts half a template silently.
- **Project side** (run in a Copier-generated project): reads `.copier-answers.yml`, fetches the template at the recorded `_commit`, converts its `copier.yml` in memory (same rules as the template side), renders it with the recorded answers to build the inventory, writes `.rootstock.toml` pointing at that same commit, and removes `.copier-answers.yml`. The project is now a rootstock project at its old version, and a plain `rootstock update` takes it to a rootstock-format version. Old template tags never need converting.
- **Both files in one root** — `copier.yml` and the repository's own `.copier-answers.yml`: both sides are converted in one run, and nothing is written until both pass *(my call, 1.3)*.

## Repository and release

- Public repository `bodasooqa/rootstock`. Created only after the owner says yes at that moment.
- Its own agent tooling is generated from harness as a Rust-only project, through Copier until rootstock can take over, then by rootstock itself.
- Published to crates.io as `rootstock` (free as of 2026-10-29). Prebuilt binaries for Linux, macOS and Windows on GitHub Releases. Homebrew later.

## Stages

1. **Core.** Template format, `new`, `update`, `status`, `check`, managed and starter files, the full update table. **Done when** tests on synthetic templates cover every row of the update table, binary files, permissions, symlinks, line endings, `when` and changed answers.
2. **Copier import and parity.** `import-copier` on both sides; also `new --subdir`, and includes resolved only against the template directory (the `include_root` key goes away) *(1.1)*; Python methods on values in templates *(1.2)*. **Done when:**
   - the latest harness release, converted, renders byte for byte identical to Copier 9.19's output for every valid combination of the answers `rust`, `frontend`, `monorepo`, `linear`, `telegram`, `orca`, `doc_form` — excluding `.copier-answers.yml` versus `.rootstock.toml`, which are compared by content (same answers, same commit);
   - updating a copy of screener from harness v0.3.3 to the latest release with rootstock (after `import-copier`) gives the same tree as `copier update`, except where Copier's known defects differ (starter files recreated after deletion, settings lost because of the `/**` render); each such difference is listed and justified.
3. **harness moves over**, in two releases:
   - **Bridge (last Copier-format minor).** Still `copier.yml`, delivered by Copier as usual. Its `harness-update.yml` learns to read the target tag: if the target ships `rootstock.toml`, it installs rootstock, runs `rootstock import-copier`, commits the import as a separate commit of the update PR *(owner decided, 1.3)*, and then runs `rootstock update --to <target>`; otherwise it runs Copier as before.
   - **Major.** `rootstock.toml` replaces `copier.yml`; `harness-update.yml` uses only rootstock (`status`, `update`) and maps exit code 2 to the `harness:conflict` label. The bridge workflow delivers it to screener as an ordinary update PR, labelled major.
   - **Order.** The major is tagged only after every known consumer has merged the bridge (today: screener, and rootstock's own repository). A consumer still on a pre-bridge workflow would be offered the major through Copier, which cannot render it; that PR still gets the `harness:major` label, which never merges itself, so the failure is visible, not silent. *(My call.)*

   Then Copier is removed from harness.

## Left to the builder

- Crate choice for git access (the `git` CLI or gix), prompts, CLI parsing, the three-way merge. The merge shells to `git merge-file`, with an empty git config, because an in-process diff3 produced different conflicts from git's *(1.3)*.
- The exact `rootstock.toml` and `.rootstock.toml` schemas, within the concepts above. They must carry a schema version.
- Caching of fetched templates.
- Error message wording.
