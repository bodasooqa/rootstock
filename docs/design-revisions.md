# rootstock: журнал ревизий ТЗ

Продолжение `docs/design.md`: по разделу на версию — что поменялось и почему. Сведённые требования — в самом ТЗ; раздел журнала нормативен только для версий, перечисленных в шапке ТЗ как не сведённые. Ревизии вносятся по решениям владельца.

---

## 1.4 · 2026-10-30 · Windows, before the first release

Merged into `docs/design.md`. The owner decided all three points before the Windows work that precedes the first release. §Constraints promised Windows builds, but nothing said how the Unix assumptions carry over.

**Hooks run in Git for Windows' `sh`.** Hooks are shell commands, and harness writes them in POSIX sh (`if command -v openspec …`). `cmd /C` would break every harness hook on Windows. rootstock needs `git` anyway, and Git for Windows ships `sh`, so a hook runs as `sh -c` everywhere. When no `sh` is found, a command that has hooks to run stops before writing.

**The executable bit and symlinks come from the git tree.** Windows file systems have no executable bit, and git checks symlinks out as plain files unless symlinks are allowed. So a harness project generated on Windows lost `+x` on every `.harness/hooks/*.sh` and committed them as `100644`. Now both are read from the template's git tree, not from the disk. Where the file system has no executable bit, rootstock sets it in the project's git index, and reads it back from there.

**Symlinks are created, or refused before writing.** Windows creates symlinks only in developer mode or as administrator. rootstock still creates real symlinks (rule 7), and when the system refuses, it stops before writing anything and says how to allow them. harness has no symlinks today.

---

## 1.3 · 2026-10-30 · Stage 2 findings, before stage 3

Merged into `docs/design.md`. Decided at the grooming that opens stage 3, on the open questions in `knowledge/open-questions.md`. The owner decided some points directly and handed the rest to the builder ("let's just do it well"); those are marked *my call*.

**The merge shells to `git merge-file`.** §Left to the builder said in-process, "but the builder may change it if conflict output differs from git's". It did: on 3000 random three-way inputs, an in-process diff3 (diffy) disagreed with `git merge-file` on 20 conflict-or-clean verdicts and on 349 conflict bytes. Copier merges with git too, so the stage 2 parity criterion needs git's output. The merge runs with an empty git config, so a user's `merge.conflictStyle` cannot change the markers.

**`check`: "inconsistent with the tree".** The text did not say what it means. The owner decided on checks that need no template checkout: the record does not parse; an inventory path is not a safe relative path; the recorded version is neither a `vX.Y.Z` tag nor `HEAD`. A managed file that is missing is not an error: the project may delete it (§How update works, row "managed | managed | absent").

**The bridge commits the import.** §Stages, item 3, said the bridge "runs `rootstock import-copier` and then `rootstock update`". But the import leaves the tree dirty, and `update` refuses a dirty tree (rule 1). The owner decided that the bridge commits the import as a separate commit of the update PR. Rule 1 stays as it is.

**Changing an answer at the current version** *(my call)*. §Commands promises that `--data` "changes an answer". But step 2 stopped at "already at the target", so an answer could not be changed without a version step. Now step 2 stops there only when no answer changes. Otherwise the update renders the same version twice, with the old answers and with the new ones.

**A Copier template that is itself a Copier project** *(my call)*. Some repositories hold both `copier.yml` and their own `.copier-answers.yml`. On those, `import-copier` refused, because it did not know which side to convert, and no flag chooses the side. Now it converts both sides in one run: each side passes its own checks, and nothing is written until both have. Afterwards such a repository holds its own `.rootstock.toml` next to `rootstock.toml`. When the template root is the template directory, that file was a render error: rootstock reserves the record's name. Now it is skipped at the top of the template directory, like `rootstock.toml` (§Template format). A template file that renders to the record's name is still an error.

**Confirmed as built, no text change** *(my call)*:
- a `blocked` path, being out of the inventory, behaves afterwards as deleted by the project;
- `--subdir` with a local path into a subdirectory is taken relative to that path.

---

## 1.2 · 2026-10-30 · Python methods in harness templates

Merged into `docs/design.md`. The owner decided it during stage 2, on a finding of `stage2-import-copier-template`.

**harness templates call Python methods that minijinja does not have.** Every harness tag from v0.3.3 to v0.4.5 has two of them:
- `names.append(name)` in `template/_partials/agents/meta.jinja`, which builds the tool list of every Pi agent;
- `open_questions_path.endswith('.md')` in `knowledge/README.md.jinja`.

Jinja2 lets a template call any Python method on a value; minijinja has none of them. So the converted harness did not render at all. With those two spots rewritten, it rendered for every combination of `rust`, `frontend` and `doc_form`, and the combination checked byte for byte matched Copier 9.19. The spec says old tags never need converting, and the stage 2 criterion renders v0.3.3 and the latest tag. So rootstock has to render these templates as they are.

Options weighed:
- a new harness release without the methods: old tags would stay unrenderable, and updating screener from v0.3.3 would be impossible;
- rewriting the methods while converting: the text of someone else's template would change silently.

The owner chose narrow Jinja2 compatibility in rootstock itself (§Template format, "Filters and tests"):
- the common string, mapping and list methods;
- `append` on a list, which in Jinja2 changes the list in place. minijinja values are immutable, so this needs rootstock's own list.

---

## 1.1 · 2026-10-30 · Stage 1 findings, before stage 2

Merged into `docs/design.md`. The owner decided every point at the grooming that opens stage 2, on the open questions stage 1 left in `knowledge/open-questions.md`.

**"Template root" had two meanings.** The text said both "`rootstock.toml` at the template root" and "template root — subdirectory holding the files", and includes resolved against "the template root". harness writes every include from the repository root (`template/_partials/…`), as Copier resolves them, so resolving against the files' subdirectory would break all of them and the converter would have to rewrite paths. Now: the directory with `rootstock.toml` is the *template directory*; includes resolve against it, and the include base is not configurable. Stage 1 code has a configuration key `include_root` beyond the spec; it goes away in stage 2.

**Subdirectory of a remote template.** §Concepts allowed "a subdirectory of" a repository, but no command said how to name one for `https://…`, `git@…` or `gh:`; stage 1 supported it only for a local path pointing into the subdirectory. Now: `new --subdir <path>` for every kind of source, kept in the record. A separate flag rather than a syntax inside the URL (`repo//sub`, as in Terraform), because `//` is legal inside URLs and every source grammar would need its own escape; cookiecutter, cargo-generate and pip keep the subdirectory apart from the address too. The flag is implemented in stage 2.

**Deviations of stage 1 from the letter, accepted:**
- A binary file or symlink the template did not change stays as the project has it (rule 6). Read literally, rule 6 reported a conflict on every update for any binary file the project had touched.
- `update` refuses with exit 1 while managed files hold markers from an earlier update (rule 1).
- Flags beyond §Commands: `update --data-file`, `new --format`, `status --format text`.
- A seventh conflict kind, `blocked` (new rule 12): the update table assumes a template path is free, and without it replacing a file by a directory stopped `update` halfway through writing.
- A case-only rename is one file only where the file system folds case and the project still has the file (§Non-goals). Pairing it everywhere overwrote a distinct user file on case-sensitive file systems; pairing never deleted the only file on case-insensitive ones.

**Where the spec was silent, the stage 1 code's choices, confirmed:** the `symlink` conflict (rule 7); refused paths and symlink targets (§Template format, "Paths and links"); what counts as binary, and a template that emits `<name>.rootstock-new` itself (rule 6); "starter → managed" with the file absent creates it, "managed → starter" with the file absent does nothing (table); `--to` may go back (rule 2); `default` and `validator` are templates, `when` an expression (§Template format); a new question with a default is an error without `--defaults` and without a terminal (rule 3); untrusted hooks with no terminal stop the command before writing (rule 10); `--git` in a non-empty directory is an error, while `new` into a non-empty directory is allowed and reports `added-by-both` (§Commands); `--version HEAD` renders untracked and ignored files too (§Commands).
