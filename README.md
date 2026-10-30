# rootstock

rootstock is a command-line tool, written in Rust and shipped as a single binary, that creates a project from a versioned git template and later updates that project to newer versions of the template without losing the project's own edits.

Status: Stage 1 (core) and Stage 2 (Copier import and parity) done; Stage 3 (harness migration) next.

## Install

```sh
cargo install rootstock --locked
```

Or download a prebuilt binary from [GitHub Releases](https://github.com/bodasooqa/rootstock/releases): Linux (x86_64, aarch64; static, musl), macOS (arm64, x86_64) and Windows (x86_64). `SHA256SUMS` lists the checksums. rootstock needs `git` on `PATH`.

## Quick start

```sh
cargo build --release          # from a checkout; binary: target/release/rootstock

rootstock new <template> <dir> [--subdir path] [--version vX.Y.Z] [--data k=v]... [--data-file f] [--defaults] [--trust] [--git] [--format text|json]
cd <dir>
rootstock status [--format text|json]
rootstock update [--to vX.Y.Z] [--data k=v]... [--data-file f] [--defaults] [--trust] [--format text|json]
rootstock check

cd <copier-template-repo>
rootstock import-copier

cd <copier-generated-project>
rootstock import-copier
```

- `new` creates the project at the given version (the latest release tag by default) and writes the record `.rootstock.toml`. `<template>` is a URL, `gh:owner/repo` or a local path; `--version HEAD` renders the working tree of a local template. `--subdir` names the template's directory inside the repository, for any kind of source; it is kept in the record, so `update` needs no flag.
- `update` merges the template changes between the recorded version and the target into the project with a three-way merge. Text is merged by `git merge-file` (so `git` must be on `PATH`), the way Copier merges, whatever the user's git config says. Conflicts are left as markers (`<<<<<<< project`, `=======`, `>>>>>>> template <version>`); binary files get a `.rootstock-new` side file next to them. `--data` (and `--data-file`) changes an answer: the base is rendered with the recorded answers, the target with the new ones. This works at the current version too: if an answer actually changes, the same version is rendered with the old and the new answers; otherwise `update` reports `already at <version>` and writes nothing.
- `update --to HEAD` on a project created from a local template always runs, since `HEAD` moves. The base of such a project is the commit recorded at `new` time, so uncommitted template edits that `new --version HEAD` rendered show up as project edits.
- `status` prints `current`, `latest` and `major` (whether the latest release is a major bump), as text or JSON.
- `check` is for CI: it lists files with unresolved conflict markers (`markers <path>`) and leftover side files (`side-file <path>`), and a record that does not match the tree: an inventory path that is not a safe relative path (`unsafe-path <path>`, never read) or a version that is neither `vX.Y.Z` nor `HEAD` (`version <value>`). It fails if there is any. It needs no network; a managed file the project deleted is not a problem.
- `import-copier`, run in a Copier template repository, converts `copier.yml` into `rootstock.toml` next to it and removes the template of Copier's answers file (`{{ _copier_conf.answers_file }}.jinja`), which the record replaces. `copier.yml` itself stays. It supports the subset of Copier that harness uses; anything else stops the import with the full list of what cannot be converted, and nothing is written.
- `import-copier`, run in a Copier-generated project (a clean git tree with `.copier-answers.yml`), takes the template at the recorded `_commit` tag, converts its `copier.yml` in memory (or uses its `rootstock.toml`, if that tag already has one), renders it with the recorded answers and writes `.rootstock.toml` at that same version; then it removes `.copier-answers.yml`. No other file changes and no hooks run. A path the template emits but the project has taken (a file where the template needs a directory) is left out of the record and listed as `blocked`, with exit code 2. Commit the result before the first `update`. A directory holding both `copier.yml` and `.copier-answers.yml` (a template that is itself a Copier project) gets both conversions in one run; nothing is written unless both pass.
- `update` also accepts template versions that still carry only `copier.yml` (base or target) and converts them in memory, so old tags never need converting. `new` does not: convert the template first.

Hooks declared by a template run only with `--trust` or after an interactive confirmation.

## Windows

rootstock builds and runs on Windows, with three things to know:

- Hooks run under `sh`. On Windows rootstock uses the `sh.exe` of [Git for Windows](https://gitforwindows.org/), found next to `git` (it looks up from `git --exec-path`). If a template has hooks to run and none is found, the command stops before writing anything. Hook commands must be POSIX shell.
- Windows files have no executable bit, so rootstock keeps it in the git index: the executable bit of a template file is read from the git tree. In a project that is a git work tree (a project in a subdirectory of a repository counts) rootstock changes only the mode of a tracked file in the index, never its content; a new executable file is added to the index (`git add --chmod=+x`). `new --git` does it right after `git init`. Paths that `.gitignore` covers, paths inside submodules and files already gone are skipped. A project that is not a git work tree gets no bit. If git refuses a path, the command finishes its work, then exits with code 1 and lists the paths.
- Symbolic links need Developer Mode (Settings, then System, then For developers) or an administrator shell. A command stops before writing anything only when the plan writes a link and the system refuses to make one.

Template files are read from the git tree with `core.autocrlf=false` and `core.eol=lf`, so line endings of a template are never changed by your git configuration. A `.gitattributes` of the template that asks for `eol=crlf` still applies.

## Template format

A template is a git repository with release tags `vX.Y.Z` and a `rootstock.toml` (`schema = 1`) in its root. A template may also be a subdirectory of a repository (`new --subdir`, or a local path pointing straight into it); then `rootstock.toml` sits in that subdirectory, and the paths in the config (`root`) as well as every `{% include %}` and `{% import %}` are relative to it; `partials` are relative to `root`. The config names the directory with the template files (`root`), the questions asked of the user, jinja options, hooks and file rules, for example which files are starter files. Files ending in `.jinja` are rendered with the answers and lose the suffix; path names may contain jinja too. Templates may call the common Python methods on values, as Jinja2 allows: string methods such as `endswith` or `split`, dict methods `items`, `keys`, `values`, `get`. `list.append(x)` changes the list in place on a list created with `{% set name = [] %}`. Managed files are kept up to date by `update`; starter files are created once and never touched again. When the template files sit in the template directory itself, a `.rootstock.toml` next to `rootstock.toml` is the repository's own record as a rootstock project and is never rendered. The full description is in `docs/design.md`.

## Exit codes

- `0` - done, nothing to resolve.
- `1` - error (bad arguments, unreadable record, failed `check`, ...).
- `2` - `new` or `update` finished with conflicts.

Stdout carries only command output (conflict lists, status, JSON); everything else goes to stderr.

## Repository map

- `src/config.rs` - `rootstock.toml` model and validation
- `src/record.rs` - `.rootstock.toml` model, inventory, reading and writing
- `src/jinja.rs` - minijinja environment, filters, expression helpers
- `src/answers.rs` - collecting answers: given, defaults, `when`, validators; asking goes through a `Prompter`
- `src/render.rs` - rendering a template checkout into a tree with file classes
- `src/merge.rs` - binary detection, conflict marker search, the merge interface
- `src/plan.rs` - the update table, `plan_update` (pure function)
- `src/source.rs` - template sources, tags, checkouts through `git`
- `src/apply.rs` - executing a plan on disk; reading the project and merging text through `git merge-file`
- `src/hooks.rs` - running hooks with a trust check
- `src/copier.rs` - converting a Copier `copier.yml`, checking the template tree for what cannot be converted, reading `.copier-answers.yml`
- `src/cli.rs`, `src/commands/{mod,new,update,status,check,import_copier}.rs` - CLI, the terminal dialog and the commands
- `tests/` - end-to-end scenarios through the binary (`cli_*.rs`), fixtures in `tests/support/`; `tests/parity/` - the Copier parity checks of Stage 2 (not run in CI)
- `docs/design.md` - the specification
- `.github/workflows/ci.yml` - tests on Linux, macOS and Windows; `release.yml` - binaries on GitHub Releases for a `vX.Y.Z` tag

## License

Licensed under either of MIT or Apache-2.0, at your option.
