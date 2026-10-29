# rootstock stage 1 (core) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A `rootstock` binary that creates a project from a versioned git template and updates it to a newer version with a three-way merge, following every row of the design's update table.

**Architecture:** One crate (lib + bin). Pure core — config, answers, rendering into an in-memory tree, three-way merge, and an update *planner* that turns (base tree, target tree, project view, inventory) into a list of actions and conflicts — with I/O kept at the edges (git source, applying a plan to disk, hooks, CLI). `new` is `update` from an empty base into an empty project.

**Tech Stack:** Rust stable, edition 2024; clap (derive), serde, toml, serde_json, minijinja, regex, ignore (gitignore matcher), semver, diffy (three-way merge), inquire (prompts), a maintained serde YAML crate, thiserror/anyhow, tempfile; tests with assert_cmd, predicates. Git through the `git` CLI (`std::process::Command`).

**Spec:** `docs/design.md` (approved 2026-10-29). Read it before any task; this plan argues from it.

## Global Constraints

- Rust stable, edition 2024, one crate named `rootstock`, binary `rootstock`. License MIT OR Apache-2.0.
- English for README, docs, CLI messages, code comments, **commit messages**. **No `Co-Authored-By` line in any commit.**
- Commit messages: conventional prefixes `feat:`, `fix:`, `test:`, `docs:`, `chore:`.
- No `unwrap()`/`expect()` outside tests; no `println!` for diagnostics (stdout carries only command output: conflict lists, status, JSON); diagnostics go to stderr.
- Record file name `.rootstock.toml`; template config `rootstock.toml`; both carry `schema = 1`.
- Templated suffix default `.jinja`; `keep_trailing_newline` default `true`; `trim_blocks`, `lstrip_blocks` default `false`.
- Exit codes: `0` clean, `1` error, `2` done with conflicts.
- Conflict markers: `<<<<<<< project`, `=======`, `>>>>>>> template <version>`, each at line start.
- Binary side file suffix `.rootstock-new`.
- Versions are tags `vX.Y.Z`; "latest" = highest non-pre-release semver tag; major bump = target major > current major (same as harness `next-version.sh`).
- Bytes in, bytes out: no line-ending conversion anywhere.

## Review Focus

1. A template path that renders to an absolute path, or one containing `..`, must be rejected with exit 1 before anything is written — otherwise a template writes outside the project. Test: Task 6.
2. CRLF files and files without a final newline must come out of a clean merge byte-identical in their line endings. Test: Task 7.
3. `update` on a project that still has unresolved markers from a previous update (committed, so the tree is "clean") must refuse with exit 1, not stack a second set of markers. Test: Task 11.
4. A recorded commit that no longer exists in the template repository (force-push) must fail with exit 1 naming the commit, and write nothing. Test: Task 9 and Task 11.
5. An answer to a question the new template version no longer has must be dropped from the new record, and must not break rendering. Test: Task 5.

---

### File Structure

| File | Responsibility |
|---|---|
| `src/config.rs` | `rootstock.toml` model and validation |
| `src/record.rs` | `.rootstock.toml` model, inventory, read/write |
| `src/jinja.rs` | minijinja environment from options, filters, expression/template helpers |
| `src/answers.rs` | collecting answers: provided, defaults, `when`, validators, prompting |
| `src/render.rs` | rendering a template checkout into a `RenderedTree` with classes |
| `src/merge.rs` | binary detection, three-way text merge with markers |
| `src/plan.rs` | the update table: `plan_update` (pure) |
| `src/source.rs` | template sources, tags, checkouts via `git` |
| `src/apply.rs` | executing a `Plan` on disk |
| `src/hooks.rs` | running hooks with trust gating |
| `src/cli.rs`, `src/commands/{new,update,status,check}.rs` | CLI and the four commands |
| `tests/support/mod.rs` | `TemplateRepo` fixture builder (git repo with tags in a tempdir) |
| `tests/cli_*.rs` | end-to-end scenarios through the binary |

Shared types live in `src/lib.rs`'s `pub mod` items as listed in each task's Interfaces. Paths inside trees are `RelPath` — a `String` with `/` separators, never starting with `/`, never containing `..` components.

---

### Task 2: Template config

**Files:** Create `src/config.rs`; Test: unit tests in the same file.

**Interfaces:**
- Produces:
  ```rust
  pub struct TemplateConfig {
      pub schema: u32, pub min_rootstock: Option<semver::Version>,
      pub root: String,            // default "."; harness uses "template"
      pub include_root: String,    // default "." = repository root (Copier's behaviour, harness's include paths rely on it)
      pub suffix: String,          // default ".jinja"
      pub partials: Vec<String>,   // dirs under root never emitted
      pub jinja: JinjaOptions,
      pub files: FileRules,        // starter: Vec<String>, exclude: Vec<String>
      pub questions: Vec<Question>,
      pub hooks: Vec<Hook>,
  }
  pub struct JinjaOptions { pub trim_blocks: bool, pub lstrip_blocks: bool, pub keep_trailing_newline: bool }
  pub struct Question { pub name: String, pub kind: QuestionKind, pub help: Option<String>,
      pub default: Option<toml::Value>, pub when: Option<String>, pub validator: Option<String>,
      pub choices: Vec<Choice> }
  pub enum QuestionKind { Bool, Str, Int, Choice, MultiChoice, Yaml }  // toml: type = "bool" | "str" | "int" | "choice" | "multichoice" | "yaml"
  pub struct Choice { pub label: String, pub value: toml::Value }        // toml: plain value, or { label, value }
  pub struct Hook { pub run: String, pub when: Vec<Phase> }
  pub enum Phase { New, Update }
  pub fn load(template_dir: &Path) -> Result<TemplateConfig, ConfigError>;
  ```
  `when` is a minijinja **expression** (no braces); `validator` and string `default`s are **templates**.

- [ ] **Step 1: Failing tests:** `parses_full_example` (every field set, choices in both forms, asserts each value), `applies_defaults` (minimal file `schema = 1` → root ".", include_root ".", suffix ".jinja", keep_trailing_newline true), `rejects_unknown_schema` (`schema = 2` → error mentioning "schema 2"), `rejects_newer_min_version` (`min_rootstock = "99.0.0"` → error naming both versions), `rejects_choice_question_without_choices`, `rejects_duplicate_question_names`.
- [ ] **Step 2:** `cargo test config::` → FAIL.
- [ ] **Step 3:** Implement with serde (`deny_unknown_fields`), validation after parse.
- [ ] **Step 4:** `cargo test config::` → PASS.
- [ ] **Step 5:** Commit `feat: template config`.

### Task 3: Project record

**Files:** Create `src/record.rs`.

**Interfaces:**
- Produces:
  ```rust
  pub type Answers = BTreeMap<String, serde_json::Value>;
  pub enum FileClass { Managed, Starter }                 // toml: "managed" | "starter"
  pub type Inventory = BTreeMap<RelPath, FileClass>;
  pub struct Record { pub schema: u32, pub source: String, pub version: String, pub commit: String,
                      pub answers: Answers, pub inventory: Inventory }
  pub const RECORD_FILE: &str = ".rootstock.toml";
  pub fn read(project: &Path) -> Result<Record, RecordError>;
  pub fn write(project: &Path, record: &Record) -> Result<(), RecordError>;
  ```

- [ ] **Step 1: Failing tests:** `round_trips` (record with nested yaml answer, both classes → write → read → equal), `writes_stable_order` (two writes of the same record are byte-identical; keys sorted), `rejects_null_answer` (answer containing `null` → error naming the question: TOML has no null), `missing_record_is_a_clear_error` (message contains ".rootstock.toml" and "not a rootstock project").
- [ ] **Step 2–4:** run → FAIL, implement, run → PASS.
- [ ] **Step 5:** Commit `feat: project record`.

### Task 4: Jinja environment

**Files:** Create `src/jinja.rs`.

**Interfaces:**
- Consumes: `JinjaOptions` (Task 2).
- Produces:
  ```rust
  pub struct Jinja { /* minijinja::Environment<'static> */ }
  impl Jinja {
      pub fn new(opts: &JinjaOptions, include_root: &Path) -> Jinja;   // loader resolves include/import names relative to include_root
      pub fn render_str(&self, name: &str, source: &str, ctx: &Answers) -> Result<String, JinjaError>;
      pub fn eval_bool(&self, expr: &str, ctx: &Answers) -> Result<bool, JinjaError>;  // expression, minijinja truthiness
  }
  ```
  Filter `regex_search(value, pattern)` → the first match as a string, or `none` when there is no match (falsy) — the semantics harness's `project_name` validator relies on.

- [ ] **Step 1: Failing tests:** `keeps_trailing_newline_by_default` ("a\n" → "a\n"), `trim_and_lstrip_blocks` (with both on, `"  {% if x %}\nA\n  {% endif %}\n"` and x=true → `"A\n"`), `regex_search_match_and_miss` (`{{ 'ab-1' | regex_search('^[a-z]+') }}` → `ab`; `{% if 'AB' | regex_search('^[a-z]+') %}y{% else %}n{% endif %}` → `n`), `include_resolves_from_include_root` (`{% include 'template/_partials/x.md.jinja' %}` with include_root = fixture repo root), `eval_bool_uses_answers` (`rust and not frontend`).
- [ ] **Step 2–4:** FAIL, implement, PASS.
- [ ] **Step 5:** Commit `feat: jinja environment`.

### Task 5: Answers

**Files:** Create `src/answers.rs`.

**Interfaces:**
- Consumes: `TemplateConfig`, `Question` (Task 2), `Answers` (Task 3), `Jinja` (Task 4).
- Produces:
  ```rust
  pub trait Prompter { fn ask(&mut self, q: &Question, default: Option<&serde_json::Value>) -> anyhow::Result<serde_json::Value>; }
  pub struct NoPrompt;        // ask() errors: "question `<name>` has no answer and no default; pass --data <name>=..."
  pub struct TerminalPrompter; // inquire-based; not unit-tested
  pub struct Collected { pub recorded: Answers, pub context: Answers }  // context also holds unasked (when=false) defaults
  pub fn parse_data(cfg: &TemplateConfig, pairs: &[String]) -> anyhow::Result<Answers>;  // "k=v", coerced by the question's type
  pub fn collect(cfg: &TemplateConfig, jinja: &Jinja, known: &Answers, use_defaults: bool,
                 prompter: &mut dyn Prompter) -> anyhow::Result<Collected>;
  ```
  Order per question: evaluate `when` against answers so far → false: context gets the rendered default, record does not. Otherwise: `known` value, else (if `use_defaults`) rendered default, else prompt. Coerce to the type; run `validator` (rendered, trimmed; non-empty = error message, prompt again if interactive, else error). `known` keys with no matching question are dropped.

- [ ] **Step 1: Failing tests:** `known_answer_wins`, `default_is_templated` (`default = "{{ project_name }}-app"`), `when_false_not_recorded_but_in_context` (harness's `tracker_prefix` with `linear=false`), `validator_message_fails_noninteractive` (harness's `project_name` validator with `Bad_Name` → error contains its message), `missing_answer_without_default_errors` (`NoPrompt`, message as above), `parse_data_coerces_types` (`rust=false` → bool, `n=3` → int, `pi_models={opus: a, sonnet: b, haiku: c}` → object), `drops_answers_to_removed_questions` (Review Focus 5).
- [ ] **Step 2–4:** FAIL, implement, PASS.
- [ ] **Step 5:** Commit `feat: answers`.

### Task 6: Rendering a tree

**Files:** Create `src/render.rs`; fixtures built in tempdirs inside the tests.

**Interfaces:**
- Consumes: Tasks 2–4.
- Produces:
  ```rust
  pub enum Entry { File { bytes: Vec<u8>, executable: bool }, Symlink { target: String } }
  pub struct Rendered { pub entry: Entry, pub class: FileClass }
  pub type RenderedTree = BTreeMap<RelPath, Rendered>;
  pub fn render_tree(repo_root: &Path, cfg: &TemplateConfig, jinja: &Jinja, ctx: &Answers) -> Result<RenderedTree, RenderError>;
  ```
  Walk `repo_root/cfg.root` (skip `.git`, skip `rootstock.toml` when root is "."). Each path component is rendered; a component that renders empty drops the entry and its subtree. Files ending in `cfg.suffix` are rendered (suffix dropped; non-UTF-8 → error naming the file); others copied byte for byte. `executable` from the source file's mode (unix; always false elsewhere). Symlinks recorded, not followed. Then: partial dirs dropped; `exclude` rules (rendered with `ctx`, empty patterns ignored, gitignore semantics via `ignore::gitignore`) drop; `starter` rules → `Starter`; rest `Managed`.

- [ ] **Step 1: Failing tests:** `conditional_names` (`{% if rust %}Cargo.toml{% endif %}` present/absent by answer), `suffix_rendered_others_verbatim` (a non-suffixed file containing `{{` is copied unchanged), `executable_bit_kept` (rendered and copied scripts), `symlink_kept`, `partials_not_emitted`, `starter_rule_with_answer` (`{% if spec_path %}{{ spec_path }}/**{% endif %}` with `spec_path=docs/spec` → `docs/spec/a.md` is Starter; with an empty `spec_path` the pattern is ignored and the file is Managed), `root_anchored_rule` (`/README.md` makes only the root README starter, not `.harness/hooks/README.md`), `rejects_record_path` (template emitting `.rootstock.toml` → error), `rejects_escaping_paths` (a component rendering to `..` or a name rendering to `/etc/x` → error, Review Focus 1).
- [ ] **Step 2–4:** FAIL, implement, PASS.
- [ ] **Step 5:** Commit `feat: render template tree`.

### Task 7: Three-way merge

**Files:** Create `src/merge.rs`.

**Interfaces:**
- Produces:
  ```rust
  pub fn is_binary(bytes: &[u8]) -> bool;   // NUL in the first 8000 bytes, or not valid UTF-8
  pub enum Merged { Clean(Vec<u8>), Conflict(Vec<u8>) }
  pub fn merge3(base: &[u8], project: &[u8], target: &[u8], target_label: &str) -> Merged;  // text only; callers check is_binary first
  pub fn has_markers(bytes: &[u8]) -> bool;   // same rule as harness conflict-markers.sh: a "<<<<<<< " line, then a "=======" line, then a ">>>>>>> " line, in order, at line start; binary → false
  ```
  Markers exactly as in Global Constraints, `target_label` = `template v1.2.0`.

- [ ] **Step 1: Failing tests:** `has_markers_needs_all_three_in_order` (a line of seven `=` alone, or a heading underline, is not a conflict; the merge output of `overlap_gives_markers` is), `non_overlapping_edits_merge_clean` (project edits line 2, template edits line 9), `overlap_gives_markers` (asserts the full output string with `<<<<<<< project` / `=======` / `>>>>>>> template v1.1.0`), `empty_base_identical_is_clean`, `empty_base_different_conflicts`, `crlf_preserved` and `no_final_newline_preserved` (Review Focus 2), `binary_detection` (NUL → true, invalid UTF-8 → true, UTF-8 text → false).
- [ ] **Step 2–4:** FAIL, implement (diffy's three-way merge; relabel its markers, or use its options if it exposes labels), PASS.
- [ ] **Step 5:** Commit `feat: three-way merge`.

### Task 8: Update planner

**Files:** Create `src/plan.rs`.

**Interfaces:**
- Consumes: `RenderedTree`, `Rendered`, `Entry` (Task 6), `Inventory`, `FileClass` (Task 3), `merge3`, `is_binary` (Task 7).
- Produces:
  ```rust
  pub trait ProjectView { fn get(&self, path: &str) -> Option<Entry>; }   // disk impl in apply.rs; tests use a BTreeMap
  pub enum Action { Write { path: RelPath, entry: Entry }, Delete { path: RelPath } }
  pub enum ConflictKind { Markers, Binary, Symlink, RemovedByTemplate, DeletedByProject, AddedByBoth }
  //  text output names: markers, binary, symlink, removed-by-template, deleted-by-project, added-by-both
  pub struct Conflict { pub path: RelPath, pub kind: ConflictKind }
  pub struct Plan { pub actions: Vec<Action>, pub conflicts: Vec<Conflict>, pub inventory: Inventory }
  pub fn plan_update(base: &RenderedTree, target: &RenderedTree, project: &dyn ProjectView,
                     inventory: &Inventory, target_label: &str) -> Plan;
  ```
  Implements the design's update table row for row, plus: binary managed files (project = base → write target; else `Write` of `<path>.rootstock-new` + `Binary` conflict); symlinks compared by target (diverged → keep project's, `Symlink` conflict); a written entry takes the target's executable bit; "added by both" with text → `Write` of the merge output + `AddedByBoth` conflict when it has markers; "starter → managed" with the project file absent → create. New inventory = every target path present after the plan, plus every target path already in the old inventory.

- [ ] **Step 1: Failing tests — one per table row**, named after it: `new_managed_absent_creates`, `new_managed_present_same_is_noop`, `new_managed_present_differs_conflicts`, `new_starter_creates`, `starter_deleted_by_project_not_recreated`, `starter_present_untouched`, `managed_unchanged_by_project_gets_target`, `managed_edited_merges`, `managed_edited_overlap_markers`, `managed_deleted_by_project_template_same_noop`, `managed_deleted_by_project_template_changed_conflicts`, `removed_by_template_unedited_deleted`, `removed_by_template_edited_conflicts`, `removed_by_template_already_absent_noop`, `managed_to_starter_untouched`, `starter_to_managed_merges`, `starter_to_managed_absent_creates`, `excluded_now_treated_as_removed`; plus `binary_diverged_writes_side_file`, `binary_unedited_replaced`, `symlink_diverged_conflicts`, `executable_bit_follows_target`, `inventory_keeps_deleted_starter`.
- [ ] **Step 2–4:** FAIL, implement, PASS.
- [ ] **Step 5:** Commit `feat: update planner`.

### Task 9: Template sources

**Files:** Create `src/source.rs`; Create `tests/support/mod.rs`.

**Interfaces:**
- Produces:
  ```rust
  pub enum Source { Git(String), Local(PathBuf) }
  pub fn parse_source(s: &str) -> Source;            // "gh:o/r" → Git("https://github.com/o/r.git"); existing dir → Local
  pub fn tags(src: &Source) -> anyhow::Result<Vec<(semver::Version, String)>>;  // git ls-remote --tags --refs; non-vX.Y.Z ignored
  pub fn latest(src: &Source) -> anyhow::Result<String>;                     // highest non-pre-release; error if none
  pub struct Checkout { pub dir: tempfile::TempDir /* or the local path for HEAD */, pub commit: String }
  pub fn checkout(src: &Source, rev: &str) -> anyhow::Result<Checkout>;      // tag or full sha; Local + "HEAD" = working tree as-is
  ```
  Test helper (`tests/support/mod.rs`, also used by Tasks 11–12):
  ```rust
  pub struct TemplateRepo { pub dir: TempDir }
  impl TemplateRepo { pub fn new() -> Self; pub fn file(&self, path: &str, content: &str) -> &Self;
      pub fn exec(&self, path: &str) -> &Self; pub fn remove(&self, path: &str) -> &Self;
      pub fn tag(&self, tag: &str) -> &Self; /* commits everything, tags */ pub fn url(&self) -> String; }
  ```

- [ ] **Step 1: Failing tests:** `parses_gh_shorthand`, `latest_skips_prereleases` (tags v1.0.0, v1.2.0, v2.0.0-rc.1, foo → v1.2.0), `checkout_by_tag_and_by_sha`, `local_head_is_working_tree` (uncommitted file visible), `missing_commit_errors` (unknown sha → error containing the sha, Review Focus 4).
- [ ] **Step 2–4:** FAIL, implement, PASS.
- [ ] **Step 5:** Commit `feat: template sources`.

### Task 10: Applying a plan and hooks

**Files:** Create `src/apply.rs`, `src/hooks.rs`.

**Interfaces:**
- Consumes: `Plan`, `Action`, `Entry`, `ProjectView` (Task 8), `Hook`, `Phase` (Task 2).
- Produces:
  ```rust
  pub struct DiskProject<'a> { pub root: &'a Path }   // impl ProjectView; ignores .git
  pub fn apply(plan: &Plan, root: &Path) -> anyhow::Result<()>;   // parents created; Delete removes now-empty parent dirs; mode set on unix
  pub fn run_hooks(hooks: &[Hook], phase: Phase, project: &Path, trust: bool, interactive: bool) -> anyhow::Result<()>;
  ```
  Hooks run with `sh -c` in the project dir, in order, stop at the first failure (error names the command and its exit status). Without `trust`: interactive → confirm listing the commands; non-interactive → error "template has hooks; rerun with --trust".

- [ ] **Step 1: Failing tests:** `apply_writes_modes_and_symlinks`, `delete_prunes_empty_dirs` (but not dirs with other files), `disk_view_reads_symlink_as_symlink`, `hooks_need_trust_noninteractive`, `hooks_run_in_order_in_project_dir` (hook writes `pwd` to a file), `failing_hook_reports_command`.
- [ ] **Step 2–4:** FAIL, implement, PASS.
- [ ] **Step 5:** Commit `feat: apply plans and run hooks`.

### Task 11: `new` and `update` commands

**Files:** Create `src/cli.rs`, `src/commands/new.rs`, `src/commands/update.rs`; Test: `tests/cli_new_update.rs`.

**Interfaces:**
- Consumes: everything above.
- Produces: `pub enum Outcome { Clean, Conflicts(Vec<Conflict>) }`, `pub fn run() -> anyhow::Result<Outcome>` (clap), flags exactly as the design's Commands section. Conflict output on stdout: one `<kind> <path>` line each (kind names from Task 8); `--format json` → `{"version":"v1.1.0","conflicts":[{"path":"…","kind":"…"}]}`.
  - `new`: dir created if missing; checkout version (default latest); collect answers; render target; plan against an empty base, `DiskProject`, empty inventory; apply; hooks (new); write record; `--git` runs `git init` only in an empty dir.
  - `--data-file <f>`: a TOML file of answers, merged under `--data` (flags win).
  - `update`: read record; require a git work tree and `git status --porcelain` empty; refuse if any inventory file `has_markers` (Task 7); target = `--to` or latest; equal to record → exit 0 "already at <v>"; checkout base (record commit) and target. Base context: `collect` on the base config with known = record answers, `use_defaults = true`, `NoPrompt`. Target context: `collect` on the target config with known = record answers + `--data-file` + `--data`, the user's `--defaults`, terminal prompter when stdin is a TTY. Render both; plan; apply; hooks (update); write record (target tag + commit, target `recorded` answers, plan inventory).

- [ ] **Step 1: Failing tests** (each builds a `TemplateRepo`, runs the binary via assert_cmd): `new_creates_project_and_record`, `new_noninteractive_missing_answer_exits_1`, `update_clean_exits_0_and_records_version`, `update_keeps_project_edit` (non-overlapping), `update_overlap_exits_2_lists_markers` (stdout `markers README.md`), `update_json_output`, `update_does_not_recreate_deleted_starter`, `update_removes_file_dropped_by_template`, `update_dirty_tree_exits_1`, `update_with_leftover_markers_exits_1` (Review Focus 3), `update_missing_base_commit_exits_1_writes_nothing` (rewrite the template repo history, Review Focus 4), `update_data_changes_answer` (`--data rust=false` drops the rust-only file), `update_to_specific_tag`.
- [ ] **Step 2–4:** FAIL, implement, PASS.
- [ ] **Step 5:** Commit `feat: new and update commands`.

### Task 12: `status` and `check` commands

**Files:** Create `src/commands/status.rs`, `src/commands/check.rs`; Test: `tests/cli_status_check.rs`.

**Interfaces:**
- Consumes: `Record` (Task 3), `has_markers` (Task 7), `tags`/`latest` (Task 9).
- Produces:
  - `status`: text `current v1.0.0\nlatest v2.0.0\nmajor true`; `--format json` → `{"current":"v1.0.0","latest":"v2.0.0","major":true}`. Current may be a `git describe` form `vX.Y.Z-N-g<sha>`: compare by its `vX.Y.Z`.
  - `check`: exit 1 listing paths when an inventory file `has_markers`, a `.rootstock-new` file exists, or the record is unreadable; else exit 0.

- [ ] **Step 1: Failing tests:** `status_reports_major`, `status_json`, `status_describe_version`, `check_clean_exits_0`, `check_markers_exits_1`, `check_side_file_exits_1`, `check_without_record_exits_1`.
- [ ] **Step 2–4:** FAIL, implement, PASS.
- [ ] **Step 5: Stage 1 acceptance:** `cargo test` all green; `cargo clippy --all-targets -- -D warnings`; every row of the design's update table has its named test in Task 8 (`cargo test plan::` lists 23 tests). README: usage of the four commands, status "stage 1 done"; `knowledge/stages.md`: `| 1. Core | закрыт |`. Commit `feat: status and check commands`, then archive the OpenSpec change per harness `change-finish`.
