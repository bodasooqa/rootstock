# rootstock

rootstock is a command-line tool, written in Rust and shipped as a single binary, that creates a project from a versioned git template and later updates that project to newer versions of the template without losing the project's own edits.

Status: Stage 1 (core) done.

## Quick start

```sh
cargo build --release          # binary: target/release/rootstock

rootstock new <template> <dir> [--subdir path] [--version vX.Y.Z] [--data k=v]... [--data-file f] [--defaults] [--trust] [--git] [--format text|json]
cd <dir>
rootstock status [--format text|json]
rootstock update [--to vX.Y.Z] [--data k=v]... [--data-file f] [--defaults] [--trust] [--format text|json]
rootstock check
```

- `new` creates the project at the given version (the latest release tag by default) and writes the record `.rootstock.toml`. `<template>` is a URL, `gh:owner/repo` or a local path; `--version HEAD` renders the working tree of a local template. `--subdir` names the template's directory inside the repository, for any kind of source; it is kept in the record, so `update` needs no flag.
- `update` merges the template changes between the recorded version and the target into the project with a three-way merge. Conflicts are left as markers (`<<<<<<< project`, `=======`, `>>>>>>> template <version>`); binary files get a `.rootstock-new` side file next to them. `--data` (and `--data-file`) changes an answer: the base is rendered with the recorded answers, the target with the new ones.
- `update --to HEAD` on a project created from a local template always runs, since `HEAD` moves. The base of such a project is the commit recorded at `new` time, so uncommitted template edits that `new --version HEAD` rendered show up as project edits.
- `status` prints `current`, `latest` and `major` (whether the latest release is a major bump), as text or JSON.
- `check` is for CI: it lists files with unresolved conflict markers (`markers <path>`) and leftover side files (`side-file <path>`) and fails if there are any.

Hooks declared by a template run only with `--trust` or after an interactive confirmation.

## Template format

A template is a git repository with release tags `vX.Y.Z` and a `rootstock.toml` (`schema = 1`) in its root. A template may also be a subdirectory of a repository (`new --subdir`, or a local path pointing straight into it); then `rootstock.toml` sits in that subdirectory, and the paths in the config (`root`) as well as every `{% include %}` and `{% import %}` are relative to it; `partials` are relative to `root`. The config names the directory with the template files (`root`), the questions asked of the user, jinja options, hooks and file rules, for example which files are starter files. Files ending in `.jinja` are rendered with the answers and lose the suffix; path names may contain jinja too. Managed files are kept up to date by `update`; starter files are created once and never touched again. The full description is in `docs/design.md`.

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
- `src/merge.rs` - binary detection, three-way text merge with markers
- `src/plan.rs` - the update table, `plan_update` (pure function)
- `src/source.rs` - template sources, tags, checkouts through `git`
- `src/apply.rs` - executing a plan on disk
- `src/hooks.rs` - running hooks with a trust check
- `src/cli.rs`, `src/commands/{mod,new,update,status,check}.rs` - CLI, the terminal dialog and the four commands
- `tests/` - end-to-end scenarios through the binary (`cli_*.rs`), fixtures in `tests/support/`
- `docs/design.md` - the specification

## License

Licensed under either of MIT or Apache-2.0, at your option.
