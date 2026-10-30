# rootstock: журнал ревизий ТЗ

Продолжение `docs/design.md`: по разделу на версию — что поменялось и почему. Сведённые требования — в самом ТЗ; раздел журнала нормативен только для версий, перечисленных в шапке ТЗ как не сведённые. Ревизии вносятся по решениям владельца.

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
