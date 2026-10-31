<div align="center">

# rootstock

**Project templates that keep growing after you use them.**

Create a project from a versioned git template, then update it to every new template version<br>
with a real three-way merge: your edits stay, the template's improvements arrive, conflicts are never silent.

[![CI](https://github.com/bodasooqa/rootstock/actions/workflows/ci.yml/badge.svg)](https://github.com/bodasooqa/rootstock/actions/workflows/ci.yml)
[![crates.io](https://img.shields.io/crates/v/rootstock.svg)](https://crates.io/crates/rootstock)
[![License: MIT OR Apache-2.0](https://img.shields.io/crates/l/rootstock.svg)](#license)

[Install](#install) · [Quick start](#quick-start) · [How updates work](#how-updates-work) · [Commands](#commands) · [Writing a template](#writing-a-template) · [Coming from Copier](#coming-from-copier)

</div>

---

In grafting, the *rootstock* is the root system, and the *scion* is the branch grafted onto it. The scion grows its own way, and the root keeps feeding it. A rootstock project works the same way. The template is the root and the project is the scion: the project goes its own way, and improvements to the template keep reaching it.

## Why rootstock

Most scaffolding tools stop once the project is created. That is fine for a one-off, but not for a template that keeps changing: shared CI, lint setups, agent tooling, in-house service skeletons. [Copier](https://copier.readthedocs.io/) solved this for Python users. rootstock brings the same model to a single static binary:

- **Three-way updates.** The old template version and the new one are both rendered with the project's answers, and only the difference is merged into the project, by `git merge-file`. Only edits that truly overlap become conflicts.
- **Nothing is lost silently.** Every case that needs a person is a conflict with exit code `2`: markers in a file, a binary side file, a file the project deleted but the template changed. CI only has to read the exit code.
- **Starter files belong to you.** Files marked *starter* are created once and never touched again. Delete one, and it stays deleted.
- **Jinja templates** via [minijinja](https://github.com/mitsuhiko/minijinja), with `trim_blocks`, `lstrip_blocks`, `keep_trailing_newline` and the common Python methods on values (`endswith`, `split`, `items`, `list.append`), so Copier template files carry over with little or no change.
- **Versioned with git tags.** A project records the tag and the exact commit. `update --to` moves to any tag, forward or back.
- **Copier import.** One command converts a Copier template or a Copier-generated project. Old template tags never need converting.
- **One binary, no runtime.** Linux, macOS and Windows. It needs only `git` on `PATH`.

## Install

```sh
cargo install rootstock --locked
```

Or download a prebuilt binary from [GitHub Releases](https://github.com/bodasooqa/rootstock/releases):

| Platform | Archive |
|---|---|
| Linux x86_64, aarch64 (static, musl) | `rootstock-vX.Y.Z-<arch>-unknown-linux-musl.tar.gz` |
| macOS arm64, x86_64 | `rootstock-vX.Y.Z-<arch>-apple-darwin.tar.gz` |
| Windows x86_64 | `rootstock-vX.Y.Z-x86_64-pc-windows-msvc.zip` |

`SHA256SUMS` in each release lists the checksums.

## Quick start

Create a project from a template: a URL, `gh:owner/repo` or a local path. The latest release tag is used by default.

```console
$ rootstock new gh:acme/service-template my-app --data project_name=my-app --defaults
created my-app from gh:acme/service-template v1.0.0
```

The project gets the rendered files and a record, `.rootstock.toml`. Commit both. The record holds the template source, the version and commit, your answers, and the inventory of every file rootstock wrote:

```toml
schema = 1
source = "gh:acme/service-template"
version = "v1.0.0"
commit = "2999e4ac2971730a73e5f9943cd1be0495faddb3"

[answers]
ci = true
project_name = "my-app"

[inventory]
".github/workflows/ci.yml" = "managed"
Makefile = "managed"
"README.md" = "starter"
```

Now the project lives its own life. Say you add a `fmt` target to the `Makefile` and make clippy stricter. Meanwhile the template releases `v1.1.0`: it bumps `actions/checkout` in CI and adds `--all-targets` to the `test` and `lint` targets.

```console
$ rootstock status
current v1.0.0
latest v1.1.0
major false

$ rootstock update
updated to v1.1.0
markers Makefile
1 conflict
```

The CI bump and the new `test` line merged on their own. Your `fmt` target is still there. The one line both sides changed is marked:

```makefile
fmt:
	cargo fmt --check

test:
	cargo test --all-targets

lint:
<<<<<<< project
	cargo clippy -- -D warnings
=======
	cargo clippy --all-targets
>>>>>>> template v1.1.0
```

Resolve the conflict, run `rootstock check` to confirm nothing is left, and commit. Every update is an ordinary diff in git, easy to review and easy to undo.

## How updates work

`update` refuses to run on a dirty tree or over unresolved markers. Then it renders two trees in temporary directories: the **base** (the recorded commit with the recorded answers) and the **target** (the new tag with the current answers). It compares both with the project, path by path.

Each path in a template has a class:

| Class | `new` | `update` |
|---|---|---|
| **managed** (default) | created | kept in sync with the template; your edits merged in |
| **starter** | created | never touched again; deleting it is permanent |
| **excluded** | not written | not written |

What happens to a managed file:

| Template changed it? | You changed it? | Result |
|---|---|---|
| no | — | left as you have it |
| yes | no | template's version written |
| yes | yes | three-way merge; overlapping lines become a `markers` conflict |
| removed it | no | deleted |
| removed it | yes | kept, reported as `removed-by-template` |
| yes | you deleted it | stays deleted, reported as `deleted-by-project` |

Binary files are never merged. If you changed one, your copy stays and the template's version goes next to it as `<name>.rootstock-new` (a `binary` conflict). Symlinks and the executable bit come from the template's git tree. Line endings are never converted: bytes in, bytes out.

Changing an answer is an update too: `rootstock update --data ci=false` renders the base with the old answers and the target with the new ones, even at the same version.

The full rules, including `added-by-both`, `symlink` and `blocked` conflicts, are in the [specification](docs/design.md#how-update-works).

## Commands

| Command | What it does |
|---|---|
| `rootstock new <template> <dir>` | create a project at a version (latest tag by default) and write the record |
| `rootstock update` | update the project in the current directory to `--to vX.Y.Z` or the latest tag |
| `rootstock status` | current version, latest available, and whether it is a major bump |
| `rootstock check` | fail on unresolved markers, leftover side files or an inconsistent record. For CI, no network |
| `rootstock import-copier` | convert a Copier template, a Copier project, or both |

Common options of `new` and `update`:

- `--data key=value` (repeatable) and `--data-file f` give answers.
- `--defaults` takes defaults for unanswered questions instead of asking.
- `--trust` allows the template's hooks. Without it, hooks need an interactive confirmation.
- `--format json` prints conflicts as JSON.

<details>
<summary><b>Full reference</b></summary>

```sh
rootstock new <template> <dir> [--subdir path] [--version vX.Y.Z] [--data k=v]... [--data-file f] [--defaults] [--trust] [--git] [--format text|json]
rootstock update [--to vX.Y.Z] [--data k=v]... [--data-file f] [--defaults] [--trust] [--format text|json]
rootstock status [--format text|json]
rootstock check
rootstock import-copier
```

- `new` creates the project at the given version (the latest release tag by default) and writes the record `.rootstock.toml`. `<template>` is a URL, `gh:owner/repo` or a local path; `--version HEAD` renders the working tree of a local template. `--subdir` names the template's directory inside the repository, for any kind of source; it is kept in the record, so `update` needs no flag. `--git` runs `git init` in a new, empty directory. A non-empty directory is allowed: a file already there that differs from the template is an `added-by-both` conflict.
- `update` merges the template changes between the recorded version and the target into the project with a three-way merge. Text is merged by `git merge-file`, the way Copier merges, whatever the user's git config says. Conflicts are left as markers (`<<<<<<< project`, `=======`, `>>>>>>> template <version>`); binary files get a `.rootstock-new` side file next to them. `--data` (and `--data-file`) changes an answer: the base is rendered with the recorded answers, the target with the new ones. This works at the current version too: if an answer actually changes, the same version is rendered with the old and the new answers; otherwise `update` reports `already at <version>` and writes nothing. `--to` may name an older tag.
- `update --to HEAD` on a project created from a local template always runs, since `HEAD` moves. The base of such a project is the commit recorded at `new` time, so uncommitted template edits that `new --version HEAD` rendered show up as project edits.
- `status` prints `current`, `latest` and `major` (whether the latest release is a major bump), as text or JSON (`{"current":"v1.0.0","latest":"v1.1.0","major":false}`).
- `check` is for CI: it lists files with unresolved conflict markers (`markers <path>`) and leftover side files (`side-file <path>`), and a record that does not match the tree: an inventory path that is not a safe relative path (`unsafe-path <path>`, never read) or a version that is neither `vX.Y.Z` nor `HEAD` (`version <value>`). It fails if there is any. It needs no network; a managed file the project deleted is not a problem.
- `import-copier` is described in [Coming from Copier](#coming-from-copier).

</details>

### Exit codes

| Code | Meaning |
|---|---|
| `0` | done, nothing to resolve |
| `1` | error: bad arguments, unreadable record, failed `check`, … Nothing written, or a clear statement of what was |
| `2` | `new`, `update` or `import-copier` finished with conflicts; they are listed one per line, `<kind> <path>` |

Stdout carries only command output (conflict lists, status, JSON); everything else goes to stderr. That makes rootstock easy to drive from CI: run `rootstock update`, open a pull request, and label it when the exit code is `2`. That is how [harness](https://github.com/bodasooqa/harness) delivers its updates to every project once it moves to rootstock.

## Writing a template

A template is a git repository with release tags `vX.Y.Z` and a `rootstock.toml` in its root:

```
service-template/
├── rootstock.toml
└── template/
    ├── README.md.jinja                              # rendered, loses the suffix
    ├── Makefile                                     # copied byte for byte
    └── {% if ci %}.github{% endif %}/workflows/
        └── ci.yml                                   # only when ci = true
```

```toml
schema = 1
root = "template"          # directory with the files to render

[[questions]]
name = "project_name"
type = "str"
help = "Project name"

[[questions]]
name = "ci"
type = "bool"
default = true
help = "Add a CI workflow?"

[files]
starter = ["README.md"]    # created once, then owned by the project
```

- **Questions** have the types `bool`, `str`, `int`, `choice`, `multichoice` and `yaml`, each with an optional `help`, `default`, `when` (asked only when the expression is true) and `validator`. `default` and `validator` are Jinja templates over earlier answers, as in Copier.
- **Files** ending in `.jinja` are rendered and lose the suffix (`suffix` changes it); everything else is copied as is. Path names are always rendered, and a name that renders empty is not written.
- **`[files]`** takes gitignore-style `starter` and `exclude` patterns. Patterns may use the answers, such as `"{{ docs_dir }}/**"`.
- **`[jinja]`** sets `trim_blocks`, `lstrip_blocks` and `keep_trailing_newline` (on by default). `{% include %}` and `{% import %}` resolve against the template directory, and `partials` lists directories that are never written.
- **`[[hooks]]`** are shell commands, `run = "…"` with `when = ["new"]`, `["update"]` or both. They run in the project after the files are written, and only with `--trust` or a confirmation.
- **`min_rootstock`** names the oldest rootstock that can render the template.

A template may also be a subdirectory of a repository (`new --subdir`). When the template files sit in the template directory itself, a `.rootstock.toml` next to `rootstock.toml` is the repository's own record as a project and is never rendered. The full format is in the [specification](docs/design.md#template-format).

## Coming from Copier

`rootstock import-copier` converts in place and writes nothing unless the whole conversion succeeds.

- **In a Copier template repository** it writes `rootstock.toml` next to `copier.yml` and removes the template of Copier's answers file (`{{ _copier_conf.answers_file }}.jinja`), since the record replaces it. `copier.yml` itself stays. It supports the subset of Copier that [harness](https://github.com/bodasooqa/harness) uses. Anything else (`_migrations`, `_jinja_extensions`, unsupported question types…) stops the import with the full list of what cannot be converted.
- **In a Copier-generated project** (a clean git tree with `.copier-answers.yml`) it takes the template at the recorded `_commit` tag, converts it in memory, and writes `.rootstock.toml` at that same version; then it removes `.copier-answers.yml`. No other file changes and no hooks run. A path the project has taken over (a file where the template needs a directory) is left out of the record and listed as `blocked`, with exit code 2. Commit the result, and the next `rootstock update` takes the project to any later version.
- **Both in one directory** (a template that is itself a Copier project): both are converted in one run.

`update` reads template versions that only carry `copier.yml` and converts them in memory, so old tags never need converting. The converted harness renders byte for byte like Copier 9.19 for every combination of its answers.

## How it compares

| | rootstock | Copier | cargo-generate | kickstart |
|---|---|---|---|---|
| Runtime | single binary | Python | single binary | single binary |
| Templates | Jinja (minijinja) | Jinja2 | Liquid | Tera |
| Updates generated projects | ✅ three-way, against the previous render | ✅ three-way, against the previous render | ❌ creation only | ❌ creation only |
| Conflicts as an exit code | ✅ `2` | ❌ markers only | — | — |

Checked in October 2026. [The specification](docs/design.md#prior-art-checked-2026-10-29) has a longer survey.

## Windows

<details>
<summary>rootstock builds and runs on Windows, with three things to know.</summary>

- Hooks run under `sh`. On Windows rootstock uses the `sh.exe` of [Git for Windows](https://gitforwindows.org/), found next to `git` (it looks up from `git --exec-path`). If a template has hooks to run and none is found, the command stops before writing anything. Hook commands must be POSIX shell.
- Windows files have no executable bit, so rootstock keeps it in the git index: the executable bit of a template file is read from the git tree. In a project that is a git work tree (a project in a subdirectory of a repository counts) rootstock changes only the mode of a tracked file in the index, never its content; a new executable file is added to the index (`git add --chmod=+x`). `new --git` does it right after `git init`. Paths that `.gitignore` covers, paths inside submodules and files already gone are skipped. A project that is not a git work tree gets no bit. If git refuses a path, the command finishes its work, then exits with code 1 and lists the paths.
- Symbolic links need Developer Mode (Settings, then System, then For developers) or an administrator shell. A command stops before writing anything only when the plan writes a link and the system refuses to make one.

Template files are read from the git tree with `core.autocrlf=false` and `core.eol=lf`, so line endings of a template are never changed by your git configuration. A `.gitattributes` of the template that asks for `eol=crlf` still applies.

</details>

## Status

`0.1` is the first public release. Stage 1 (the core) and Stage 2 (Copier import and parity) are done. Stage 3 is in progress: [harness](https://github.com/bodasooqa/harness) moves from Copier to rootstock, and its daily update pull requests keep working as they do now. Homebrew is planned.

## Development

```sh
cargo build --release   # binary: target/release/rootstock
cargo test
cargo clippy --all-targets --all-features -- -D warnings
cargo fmt --check
```

The specification is [`docs/design.md`](docs/design.md); it is the source of truth for behaviour.

<details>
<summary><b>Repository map</b></summary>

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

</details>

## License

Licensed under either of [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at your option.
