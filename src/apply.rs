//! Executes a `Plan` on disk and reads the project back as a `ProjectView`.

use std::cell::{Cell, RefCell};
use std::path::Path;

use crate::RelPath;
use crate::merge::{Merge3, Merged};
use crate::plan::{Action, Plan, ProjectView};
use crate::render::{Entry, FileMode, Modes, target_from_os, target_to_os};

/// The project directory as a `ProjectView`; `.git` is invisible.
pub struct DiskProject<'a> {
    pub root: &'a Path,
    errors: RefCell<Vec<String>>,
    /// Modes in the git index of the project; filled only where the file system
    /// keeps no executable bit (see `load_index`).
    index: Modes,
}

impl<'a> DiskProject<'a> {
    pub fn new(root: &'a Path) -> Self {
        let mut errors = Vec::new();
        let index = load_index(root).unwrap_or_else(|e| {
            errors.push(format!("cannot read the git index: {e:#}"));
            Modes::new()
        });
        Self {
            root,
            errors: RefCell::new(errors),
            index,
        }
    }

    /// Returns the read errors recorded by `get`, if any.
    pub fn finish(self) -> anyhow::Result<()> {
        let errors = self.errors.into_inner();
        if errors.is_empty() {
            Ok(())
        } else {
            anyhow::bail!("cannot read the project: {}", errors.join("; "))
        }
    }

    fn record(&self, message: String) {
        self.errors.borrow_mut().push(message);
    }

    /// Runs the parent guard for `path`: `Ok(false)` when a parent is missing or
    /// is not a directory (the path is absent), an error for a symlinked parent.
    fn parents_present(&self, path: &str) -> Result<bool, String> {
        readable_parents(self.root, path).map_err(|e| e.to_string())
    }

    fn read(&self, path: &str) -> Result<Option<Entry>, String> {
        let full = self.root.join(path);
        if !self.parents_present(path)? {
            return Ok(None);
        }
        let meta = match std::fs::symlink_metadata(&full) {
            Ok(meta) => meta,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(format!("`{path}`: {e}")),
        };
        let file_type = meta.file_type();
        if file_type.is_dir() {
            // A directory where the template has a file: the plan sees it through `list_files`.
            return Ok(None);
        }
        if file_type.is_symlink() {
            let target = std::fs::read_link(&full).map_err(|e| format!("`{path}`: {e}"))?;
            let target = target
                .into_os_string()
                .into_string()
                .map_err(|_| format!("`{path}`: symlink target is not valid UTF-8"))?;
            return Ok(Some(Entry::Symlink {
                target: target_from_os(&target),
            }));
        }
        if !file_type.is_file() {
            return Err(format!("`{path}` is a special file, expected a file"));
        }
        let bytes = std::fs::read(&full).map_err(|e| format!("`{path}`: {e}"))?;
        let indexed = self.index.get(path).copied();
        if indexed == Some(FileMode::Symlink) {
            // Git without the right to make links keeps a link as a file with the target.
            let target = String::from_utf8(bytes)
                .map_err(|_| format!("`{path}`: symlink target is not valid UTF-8"))?;
            return Ok(Some(Entry::Symlink { target }));
        }
        Ok(Some(Entry::File {
            bytes,
            executable: is_executable(&meta) || indexed == Some(FileMode::Executable),
        }))
    }

    fn list(&self, dir: &str) -> Result<Option<Vec<RelPath>>, String> {
        if !self.parents_present(dir)? {
            return Ok(None);
        }
        match std::fs::symlink_metadata(self.root.join(dir)) {
            Ok(meta) if meta.is_dir() => {}
            Ok(_) => return Ok(None),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(format!("`{dir}`: {e}")),
        }
        let mut out = Vec::new();
        let mut pending = vec![dir.to_string()];
        while let Some(current) = pending.pop() {
            let entries = std::fs::read_dir(self.root.join(&current))
                .map_err(|e| format!("`{current}`: {e}"))?;
            let mut empty = true;
            for item in entries {
                empty = false;
                let item = item.map_err(|e| format!("`{current}`: {e}"))?;
                let name = item
                    .file_name()
                    .into_string()
                    .map_err(|_| format!("`{current}`: a file name is not valid UTF-8"))?;
                let rel = format!("{current}/{name}");
                // `file_type` does not follow symlinks.
                let file_type = item.file_type().map_err(|e| format!("`{rel}`: {e}"))?;
                if file_type.is_dir() {
                    pending.push(rel);
                } else {
                    out.push(rel);
                }
            }
            // An empty directory below `dir` is an occupant no plan deletes:
            // `get` never sees a directory, so no `Delete` names it.
            if empty && current != dir {
                out.push(current);
            }
        }
        Ok(Some(out))
    }
}

#[cfg(unix)]
fn is_executable(meta: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;
    meta.permissions().mode() & 0o111 != 0
}

#[cfg(not(unix))]
fn is_executable(_meta: &std::fs::Metadata) -> bool {
    false
}

/// Runs git for the project at `root`: paths are literal (no globs from file
/// names) and `safecrlf` cannot turn a line-ending warning into a failure.
fn run_git_in(root: &Path, args: &[&str]) -> anyhow::Result<std::process::Output> {
    let dir = root.to_string_lossy();
    let mut all = vec![
        "--literal-pathspecs",
        "-c",
        "core.safecrlf=false",
        "-C",
        &*dir,
    ];
    all.extend(args);
    crate::source::run_git(&all)
}

/// True when `root` is inside a git work tree (its own or a repository's
/// above it, as in a monorepo). A directory that does not exist is not.
fn is_git_work_tree(root: &Path) -> anyhow::Result<bool> {
    let out = run_git_in(root, &["rev-parse", "--is-inside-work-tree"])?;
    Ok(out.status.success() && String::from_utf8_lossy(&out.stdout).trim() == "true")
}

/// The modes in the project's git index. Where the file system keeps the
/// executable bit (Unix) the disk is the truth and nothing is read.
#[cfg(unix)]
fn load_index(_root: &Path) -> anyhow::Result<Modes> {
    Ok(Modes::new())
}

#[cfg(not(unix))]
fn load_index(root: &Path) -> anyhow::Result<Modes> {
    read_index(root)
}

/// Reads `git ls-files -s -z` of the project (paths relative to it); empty
/// when it is no git work tree. Read on Unix only by tests.
#[cfg(any(not(unix), test))]
fn read_index(root: &Path) -> anyhow::Result<Modes> {
    Ok(index_entries(root)?
        .into_iter()
        .filter_map(|entry| {
            let mode = match entry.kind {
                IndexKind::File => FileMode::File,
                IndexKind::Executable => FileMode::Executable,
                IndexKind::Symlink => FileMode::Symlink,
                IndexKind::Gitlink => return None,
            };
            Some((entry.path, mode))
        })
        .collect())
}

/// What an index entry is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum IndexKind {
    File,
    Executable,
    Symlink,
    /// A submodule (`160000`).
    Gitlink,
}

/// One line of `git ls-files -s -z`.
#[derive(Debug, Clone, PartialEq, Eq)]
struct IndexEntry {
    kind: IndexKind,
    sha: String,
    /// Merge stage; `0` unless the path is unmerged.
    stage: u8,
    path: RelPath,
}

/// Parses `git ls-files -s -z` (`<mode> <sha> <stage>\t<path>\0`). Lines that
/// do not fit, and paths that are not UTF-8, are left out.
fn parse_index(listing: &[u8]) -> Vec<IndexEntry> {
    let mut entries = Vec::new();
    for record in listing.split(|b| *b == 0) {
        let Some(tab) = record.iter().position(|b| *b == b'\t') else {
            continue;
        };
        let (Ok(meta), Ok(path)) = (
            std::str::from_utf8(&record[..tab]),
            std::str::from_utf8(&record[tab + 1..]),
        ) else {
            continue;
        };
        let mut fields = meta.split(' ');
        let (Some(mode), Some(sha), Some(stage)) = (fields.next(), fields.next(), fields.next())
        else {
            continue;
        };
        let kind = match mode {
            "160000" => IndexKind::Gitlink,
            "120000" => IndexKind::Symlink,
            "100755" => IndexKind::Executable,
            _ => IndexKind::File,
        };
        entries.push(IndexEntry {
            kind,
            sha: sha.to_string(),
            stage: stage.parse().unwrap_or(0),
            path: path.to_string(),
        });
    }
    entries
}

/// The entries of the project's index; empty when it is no git work tree.
fn index_entries(root: &Path) -> anyhow::Result<Vec<IndexEntry>> {
    if !is_git_work_tree(root)? {
        return Ok(Vec::new());
    }
    let out = run_git_in(root, &["ls-files", "-s", "-z"])?;
    if !out.status.success() {
        anyhow::bail!(
            "git ls-files failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(parse_index(&out.stdout))
}

/// Git modes the index records for a regular file.
const MODE_FILE: &str = "100644";
const MODE_EXECUTABLE: &str = "100755";

/// One change `set_index_modes` makes to the git index.
#[derive(Debug, Clone, PartialEq, Eq)]
enum IndexOp {
    /// A tracked file changes mode; the content in the index stays.
    SetMode {
        path: RelPath,
        mode: &'static str,
        sha: String,
    },
    /// A tracked link is replaced by a file: the content comes from the disk.
    ReplaceLink { path: RelPath, mode: &'static str },
    /// An untracked executable file is added with its bit.
    AddExecutable { path: RelPath },
}

impl IndexOp {
    fn path(&self) -> &str {
        match self {
            IndexOp::SetMode { path, .. }
            | IndexOp::ReplaceLink { path, .. }
            | IndexOp::AddExecutable { path } => path,
        }
    }
}

/// Chooses the index changes for the files a plan writes (`(path, executable)`).
/// `exists` says whether a path is still on the disk, `ignored` whether
/// `.gitignore` covers it. Skipped: missing and ignored paths, paths inside a
/// submodule (its index is not ours), paths with unmerged entries.
fn plan_index_ops(
    files: &[(&str, bool)],
    index: &[IndexEntry],
    exists: &dyn Fn(&str) -> bool,
    ignored: &dyn Fn(&str) -> bool,
) -> Vec<IndexOp> {
    let tracked: std::collections::BTreeMap<&str, &IndexEntry> = index
        .iter()
        .map(|entry| (entry.path.as_str(), entry))
        .collect();
    let unmerged = |path: &str| index.iter().any(|e| e.stage != 0 && e.path == path);
    let in_submodule = |path: &str| {
        index.iter().any(|e| {
            e.kind == IndexKind::Gitlink
                && path
                    .strip_prefix(e.path.as_str())
                    .is_some_and(|rest| rest.starts_with('/'))
        })
    };
    let mode_of = |executable: bool| {
        if executable {
            MODE_EXECUTABLE
        } else {
            MODE_FILE
        }
    };
    let mut ops = Vec::new();
    for &(path, executable) in files {
        if !exists(path) || in_submodule(path) || unmerged(path) {
            continue;
        }
        let op = match tracked.get(path) {
            Some(entry) => match entry.kind {
                IndexKind::File if executable => IndexOp::SetMode {
                    path: path.to_string(),
                    mode: MODE_EXECUTABLE,
                    sha: entry.sha.clone(),
                },
                IndexKind::Executable if !executable => IndexOp::SetMode {
                    path: path.to_string(),
                    mode: MODE_FILE,
                    sha: entry.sha.clone(),
                },
                IndexKind::Symlink => IndexOp::ReplaceLink {
                    path: path.to_string(),
                    mode: mode_of(executable),
                },
                _ => continue,
            },
            None if executable => IndexOp::AddExecutable {
                path: path.to_string(),
            },
            None => continue,
        };
        if !ignored(path) {
            ops.push(op);
        }
    }
    ops
}

/// True when `.gitignore` rules cover `path` of the project at `root`.
fn is_ignored(root: &Path, path: &str) -> bool {
    // `check-ignore` takes paths, not pathspecs, and refuses `--literal-pathspecs`.
    let dir = root.to_string_lossy();
    crate::source::run_git(&["-C", &dir, "check-ignore", "-q", "--", path])
        .map(|out| out.status.success())
        .unwrap_or(false)
}

/// The path of the project below the repository's top, `/`-terminated
/// (`pkg/` for a project in a monorepo, empty at the top). `update-index
/// --cacheinfo` takes paths from the top, unlike the other commands.
fn repository_prefix(root: &Path) -> anyhow::Result<String> {
    let out = run_git_in(root, &["rev-parse", "--show-prefix"])?;
    if !out.status.success() {
        anyhow::bail!(
            "git rev-parse --show-prefix failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Runs one git command of an index change; the error carries git's reason.
fn run_index_git(root: &Path, args: &[&str]) -> anyhow::Result<String> {
    let out = run_git_in(root, args)?;
    if !out.status.success() {
        anyhow::bail!("{}", String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Carries out the index changes. A failing path is noted (`` `path`: reason ``)
/// and the others still run.
fn run_index_ops(root: &Path, prefix: &str, ops: &[IndexOp]) -> Vec<String> {
    let mut failed = Vec::new();
    for op in ops {
        let result = match op {
            IndexOp::SetMode { path, mode, sha } => run_index_git(
                root,
                &[
                    "update-index",
                    "--cacheinfo",
                    mode,
                    sha.as_str(),
                    &format!("{prefix}{path}"),
                ],
            )
            .map(|_| ()),
            IndexOp::ReplaceLink { path, mode } => {
                run_index_git(root, &["hash-object", "-w", "--", path]).and_then(|sha| {
                    run_index_git(
                        root,
                        &[
                            "update-index",
                            "--cacheinfo",
                            mode,
                            &sha,
                            &format!("{prefix}{path}"),
                        ],
                    )
                    .map(|_| ())
                })
            }
            IndexOp::AddExecutable { path } => {
                run_index_git(root, &["add", "--chmod=+x", "--", path]).map(|_| ())
            }
        };
        if let Err(e) = result {
            failed.push(format!("`{}`: {e}", op.path()));
        }
    }
    failed
}

/// Records the executable bit of the files `plan` writes in the git index of
/// `root`, where the file system has no bit of its own (Windows). Returns the
/// paths that git refused, with the reason; the caller reports them after its
/// own work is done. Does nothing on Unix and outside a git work tree.
pub fn set_index_modes(plan: &Plan, root: &Path) -> anyhow::Result<Vec<String>> {
    if cfg!(unix) {
        return Ok(Vec::new());
    }
    set_index_modes_in(plan, root)
}

/// `set_index_modes` on every OS, so that the git side is tested on Unix too.
fn set_index_modes_in(plan: &Plan, root: &Path) -> anyhow::Result<Vec<String>> {
    if !is_git_work_tree(root)? {
        return Ok(Vec::new());
    }
    let files: Vec<(&str, bool)> = plan
        .actions
        .iter()
        .filter_map(|action| match action {
            Action::Write {
                path,
                entry: Entry::File { executable, .. },
            } => Some((path.as_str(), *executable)),
            _ => None,
        })
        .collect();
    let ops = plan_index_ops(
        &files,
        &index_entries(root)?,
        &|path| std::fs::symlink_metadata(root.join(path)).is_ok(),
        &|path| is_ignored(root, path),
    );
    Ok(run_index_ops(root, &repository_prefix(root)?, &ops))
}

impl ProjectView for DiskProject<'_> {
    fn get(&self, path: &str) -> Option<Entry> {
        if let Err(e) = check_path(path) {
            self.record(e.to_string());
            return None;
        }
        if path.split('/').next() == Some(".git") {
            return None;
        }
        match self.read(path) {
            Ok(entry) => entry,
            Err(message) => {
                self.record(message);
                None
            }
        }
    }

    fn list_files(&self, dir: &str) -> Option<Vec<RelPath>> {
        if let Err(e) = check_path(dir) {
            self.record(e.to_string());
            return None;
        }
        if dir.split('/').next() == Some(".git") {
            return None;
        }
        match self.list(dir) {
            Ok(files) => files,
            Err(message) => {
                self.record(message);
                None
            }
        }
    }
}

/// Applies the plan's actions in order under `root`.
pub fn apply(plan: &Plan, root: &Path) -> anyhow::Result<()> {
    let total = plan.actions.len();
    for (done, action) in ordered(&plan.actions, cfg!(not(unix)))
        .into_iter()
        .enumerate()
    {
        let path = match action {
            Action::Write { path, .. } | Action::Delete { path } => path,
        };
        apply_one(action, root).map_err(|e| {
            anyhow::anyhow!("applied {done} of {total} changes; failed at `{path}`: {e:#}")
        })?;
    }
    Ok(())
}

fn is_symlink_write(action: &Action) -> bool {
    matches!(
        action,
        Action::Write {
            entry: Entry::Symlink { .. },
            ..
        }
    )
}

/// The actions in the order they run: as planned, or with link writes moved
/// behind the rest. A link whose target is another link of the plan (or lies
/// below it) comes after that link; otherwise their order is kept.
fn ordered(actions: &[Action], links_last: bool) -> Vec<&Action> {
    if !links_last {
        return actions.iter().collect();
    }
    let (links, rest): (Vec<&Action>, Vec<&Action>) =
        actions.iter().partition(|action| is_symlink_write(action));
    rest.into_iter().chain(order_links(links)).collect()
}

/// The project path a link at `link` with relative `target` points to,
/// lexically; `None` when it climbs out of the project.
fn resolved_target(link: &str, target: &str) -> Option<String> {
    let mut parts: Vec<&str> = link.split('/').collect();
    parts.pop();
    for part in target.split(['/', '\\']) {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            other => parts.push(other),
        }
    }
    Some(parts.join("/"))
}

/// Sorts link writes so each follows the links its target runs through.
fn order_links(links: Vec<&Action>) -> Vec<&Action> {
    let parts = |action: &Action| -> Option<(String, Option<String>)> {
        match action {
            Action::Write {
                path,
                entry: Entry::Symlink { target },
            } => Some((path.clone(), resolved_target(path, target))),
            _ => None,
        }
    };
    let depends = |a: &Action, b: &Action| -> bool {
        match (parts(a), parts(b)) {
            (Some((_, Some(resolved))), Some((other, _))) => {
                resolved == other
                    || resolved
                        .strip_prefix(other.as_str())
                        .is_some_and(|rest| rest.starts_with('/'))
            }
            _ => false,
        }
    };
    let mut pending = links;
    let mut out = Vec::with_capacity(pending.len());
    while !pending.is_empty() {
        // A cycle has no first link: take the oldest one.
        let next = pending
            .iter()
            .position(|a| {
                !pending
                    .iter()
                    .any(|b| !std::ptr::eq(*a, *b) && depends(a, b))
            })
            .unwrap_or(0);
        out.push(pending.remove(next));
    }
    out
}

/// The refusal printed when the system will not make a link.
fn symlink_refusal(windows: bool, error: &std::io::Error) -> anyhow::Error {
    let text = if windows {
        "cannot create symlinks: enable Developer Mode in Windows settings, or run as administrator"
    } else {
        "cannot create symlinks on this file system"
    };
    anyhow::anyhow!("{text} ({error})")
}

/// Fails before anything is written when the plan writes a link and the
/// system will not make one. The probe is a link made in a scratch directory
/// inside the nearest existing ancestor of `root` (the project may not exist
/// yet, and link support depends on the volume) and removed again.
pub fn check_symlinks_allowed(plan: &Plan, root: &Path) -> anyhow::Result<()> {
    if !plan.actions.iter().any(is_symlink_write) {
        return Ok(());
    }
    let ancestor = root
        .ancestors()
        // A relative root ends in the empty path: the current directory.
        .map(|dir| {
            if dir.as_os_str().is_empty() {
                Path::new(".")
            } else {
                dir
            }
        })
        .find(|dir| dir.is_dir())
        .ok_or_else(|| anyhow::anyhow!("no existing directory above `{}`", root.display()))?;
    let scratch = tempfile::Builder::new().tempdir_in(ancestor)?;
    // The target need not exist; a file link is the kind the system must allow.
    make_link("probe-target", &scratch.path().join("probe"), false)
        .map_err(|e| symlink_refusal(cfg!(windows), &e))
}

/// True for a link to a directory on Windows, which only `remove_dir` removes.
#[cfg(windows)]
fn is_dir_link(meta: &std::fs::Metadata) -> bool {
    use std::os::windows::fs::FileTypeExt;
    meta.file_type().is_symlink_dir()
}

#[cfg(not(windows))]
fn is_dir_link(_meta: &std::fs::Metadata) -> bool {
    false
}

/// Whether a link at `parent/<name>` to `target` points at a directory now
/// (a dangling link counts as a file). Windows makes the two kinds apart.
fn link_is_dir(parent: &Path, target: &str) -> bool {
    parent.join(target).is_dir()
}

fn apply_one(action: &Action, root: &Path) -> anyhow::Result<()> {
    match action {
        Action::Write { path, entry } => {
            check_path(path)?;
            let full = root.join(path);
            let parent = create_parents(root, path)?;
            if let Ok(meta) = std::fs::symlink_metadata(&full)
                && is_dir_link(&meta)
            {
                // A directory link gives way to the new entry; it is no directory to the plan.
                std::fs::remove_dir(&full)?;
            }
            if let Ok(meta) = std::fs::symlink_metadata(&full)
                && meta.is_dir()
                // Deletes run first: a directory they emptied gives way to the file.
                && std::fs::remove_dir(&full).is_err()
            {
                anyhow::bail!("a directory exists at this path");
            }
            match entry {
                Entry::File { bytes, executable } => write_file(&parent, &full, bytes, *executable),
                Entry::Symlink { target } => write_symlink(&parent, &full, target),
            }
        }
        Action::Delete { path } => {
            check_path(path)?;
            let full = root.join(path);
            if existing_parents(root, path, "refusing to delete through symlink")?.is_none() {
                return Ok(());
            }
            match std::fs::symlink_metadata(&full) {
                Ok(meta) if meta.is_dir() => anyhow::bail!("a directory exists at this path"),
                Ok(meta) => {
                    if is_dir_link(&meta) {
                        std::fs::remove_dir(&full)?;
                    } else {
                        std::fs::remove_file(&full)?;
                    }
                    prune_empty_parents(root, &full);
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(e.into()),
            }
            Ok(())
        }
    }
}

/// How `walk_parents` treats a missing or non-directory component.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Walk {
    /// Create missing components; a non-directory is an error.
    Create,
    /// A missing component yields `None`; a non-directory is an error.
    Existing,
    /// A missing or non-directory component yields `None`.
    Read,
}

/// Walks the parent components of `path` below `root`, refusing symlinks;
/// missing and non-directory components are handled per `mode`.
fn walk_parents(
    root: &Path,
    path: &str,
    mode: Walk,
    symlink_message: &str,
) -> anyhow::Result<Option<std::path::PathBuf>> {
    let mut current = root.to_path_buf();
    let mut components: Vec<&str> = path.split('/').collect();
    components.pop();
    for component in components {
        current.push(component);
        match std::fs::symlink_metadata(&current) {
            Ok(meta) if meta.file_type().is_symlink() => {
                anyhow::bail!("{symlink_message} `{}`", current.display())
            }
            Ok(meta) if !meta.is_dir() => {
                if mode == Walk::Read {
                    return Ok(None);
                }
                anyhow::bail!("`{}` is not a directory", current.display())
            }
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                if mode != Walk::Create {
                    return Ok(None);
                }
                std::fs::create_dir(&current)?;
            }
            Err(e) => return Err(e.into()),
        }
    }
    Ok(Some(current))
}

/// Walks and, where missing, creates the parents of `path`; returns the parent directory.
fn create_parents(root: &Path, path: &str) -> anyhow::Result<std::path::PathBuf> {
    let mut parent = root.to_path_buf();
    let mut components: Vec<&str> = path.split('/').collect();
    components.pop();
    parent.extend(components);
    walk_parents(
        root,
        path,
        Walk::Create,
        "refusing to write through symlink",
    )?;
    Ok(parent)
}

/// Walks existing parents only; `None` means a component is missing.
pub(crate) fn existing_parents(
    root: &Path,
    path: &str,
    symlink_message: &str,
) -> anyhow::Result<Option<()>> {
    Ok(walk_parents(root, path, Walk::Existing, symlink_message)?.map(|_| ()))
}

/// Walks parents the way `DiskProject` reads: `false` when a component is
/// missing or not a directory, an error for a symlinked one.
pub(crate) fn readable_parents(root: &Path, path: &str) -> anyhow::Result<bool> {
    match walk_parents(root, path, Walk::Read, "path goes through symlink") {
        Ok(found) => Ok(found.is_some()),
        Err(e) => anyhow::bail!("`{path}`: {e}"),
    }
}

/// Rejects paths that could leave the root or mean something else on Windows:
/// empty, absolute, any `\`, a `:` in a segment (drive prefix, alternate data
/// stream), or `""`, `.`, `..` segments.
pub(crate) fn check_path(path: &str) -> anyhow::Result<()> {
    let bad = path.is_empty()
        || path.starts_with('/')
        || path.contains('\\')
        || path
            .split('/')
            .any(|s| matches!(s, "" | "." | "..") || s.contains(':'));
    if bad {
        anyhow::bail!("unsafe path `{path}`");
    }
    Ok(())
}

fn write_file(parent: &Path, full: &Path, bytes: &[u8], executable: bool) -> anyhow::Result<()> {
    use std::io::Write;
    // A regular file being replaced lends its permission bits to the new one.
    let existing = std::fs::symlink_metadata(full)
        .ok()
        .filter(|meta| meta.file_type().is_file());
    let mut tmp = tempfile::NamedTempFile::new_in(parent)?;
    tmp.write_all(bytes)?;
    set_mode(tmp.path(), executable, existing.as_ref())?;
    tmp.persist(full).map_err(|e| e.error)?;
    Ok(())
}

/// A new file gets 0o644 or 0o755. A replaced file keeps its permission
/// bits, except that each executable bit is set where its read bit is, or
/// cleared. setuid, setgid and sticky never carry over onto template content.
#[cfg(unix)]
fn set_mode(
    path: &Path,
    executable: bool,
    existing: Option<&std::fs::Metadata>,
) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mode = match existing {
        Some(meta) => {
            let kept = meta.permissions().mode() & 0o777 & !0o111;
            if executable {
                kept | ((kept & 0o444) >> 2)
            } else {
                kept
            }
        }
        None if executable => 0o755,
        None => 0o644,
    };
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
}

#[cfg(not(unix))]
fn set_mode(
    _path: &Path,
    _executable: bool,
    _existing: Option<&std::fs::Metadata>,
) -> std::io::Result<()> {
    Ok(())
}

fn write_symlink(parent: &Path, full: &Path, target: &str) -> anyhow::Result<()> {
    // The link is built inside a fresh temp dir, then renamed over the path.
    let scratch = tempfile::Builder::new().tempdir_in(parent)?;
    let link = scratch.path().join("link");
    make_link(&target_to_os(target), &link, link_is_dir(parent, target))?;
    std::fs::rename(&link, full)?;
    Ok(())
}

#[cfg(unix)]
fn make_link(target: &str, link: &Path, _dir: bool) -> std::io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}

/// Windows has a directory link and a file link; `dir` picks the kind.
#[cfg(windows)]
fn make_link(target: &str, link: &Path, dir: bool) -> std::io::Result<()> {
    if dir {
        std::os::windows::fs::symlink_dir(target, link)
    } else {
        std::os::windows::fs::symlink_file(target, link)
    }
}

#[cfg(not(any(unix, windows)))]
fn make_link(_target: &str, _link: &Path, _dir: bool) -> std::io::Result<()> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "symlinks are not supported on this platform",
    ))
}

/// Removes empty directories above `full`, up to but not including `root`.
fn prune_empty_parents(root: &Path, full: &Path) {
    let mut dir = full.parent();
    while let Some(current) = dir {
        if current == root || !current.starts_with(root) {
            break;
        }
        if std::fs::remove_dir(current).is_err() {
            break;
        }
        dir = current.parent();
    }
}

/// `git merge-file` exits with the number of conflicts, capped at this value.
const MAX_CONFLICT_EXIT: i32 = 127;

/// Why a `git merge-file` run failed; both carry the message.
enum Failure {
    /// The process did not start (reported once, for the first file).
    Spawn(String),
    Exit(String),
}

/// Three-way text merge through `git merge-file`; errors are collected for `finish`.
pub struct GitMerge {
    program: String,
    errors: RefCell<Vec<String>>,
    spawn_failed: Cell<bool>,
}

impl Default for GitMerge {
    fn default() -> Self {
        Self::new()
    }
}

impl GitMerge {
    pub fn new() -> Self {
        Self {
            program: "git".to_string(),
            errors: RefCell::new(Vec::new()),
            spawn_failed: Cell::new(false),
        }
    }

    #[cfg(test)]
    fn with_program(program: &str) -> Self {
        Self {
            program: program.to_string(),
            errors: RefCell::new(Vec::new()),
            spawn_failed: Cell::new(false),
        }
    }

    /// Returns the merge errors recorded by `merge3`, if any.
    pub fn finish(self) -> anyhow::Result<()> {
        let errors = self.errors.into_inner();
        if errors.is_empty() {
            Ok(())
        } else {
            anyhow::bail!("cannot merge: {}", errors.join("; "))
        }
    }

    fn run(
        &self,
        path: &str,
        base: &[u8],
        project: &[u8],
        target: &[u8],
        target_label: &str,
    ) -> Result<Merged, Failure> {
        let dir = tempfile::tempdir()
            .map_err(|e| Failure::Exit(format!("`{path}`: cannot create a temp dir: {e}")))?;
        let write = |name: &str, bytes: &[u8]| {
            std::fs::write(dir.path().join(name), bytes)
                .map_err(|e| Failure::Exit(format!("`{path}`: cannot write {name}: {e}")))
        };
        write("project", project)?;
        write("base", base)?;
        write("target", target)?;
        // An empty git config keeps the user's `merge.*` settings out of the result.
        write("gitconfig", b"")?;
        let out = crate::source::git_command(&self.program)
            .current_dir(dir.path())
            .args(["-c", "merge.conflictStyle=merge"])
            .args(["merge-file", "-p", "-L", "project", "-L", "base", "-L"])
            .arg(target_label)
            .args(["project", "base", "target"])
            .env("GIT_CONFIG_GLOBAL", dir.path().join("gitconfig"))
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .output()
            .map_err(|e| {
                Failure::Spawn(format!(
                    "`{path}`: failed to run {} merge-file: {e}",
                    self.program
                ))
            })?;
        match out.status.code() {
            Some(0) => Ok(Merged::Clean(out.stdout)),
            Some(n) if (1..=MAX_CONFLICT_EXIT).contains(&n) => Ok(Merged::Conflict(out.stdout)),
            other => Err(Failure::Exit(format!(
                "`{path}`: git merge-file failed ({}): {}",
                other.map_or("killed by a signal".to_string(), |c| format!("exit {c}")),
                String::from_utf8_lossy(&out.stderr).trim()
            ))),
        }
    }
}

impl Merge3 for GitMerge {
    fn merge3(
        &self,
        path: &str,
        base: &[u8],
        project: &[u8],
        target: &[u8],
        target_label: &str,
    ) -> Merged {
        // Trivial cases never need git.
        if project == target || base == target {
            return Merged::Clean(project.to_vec());
        }
        if base == project {
            return Merged::Clean(target.to_vec());
        }
        match self.run(path, base, project, target, target_label) {
            Ok(merged) => merged,
            Err(failure) => {
                let message = match failure {
                    // A missing git fails every file alike: say it once.
                    Failure::Spawn(_) if self.spawn_failed.replace(true) => None,
                    Failure::Spawn(message) | Failure::Exit(message) => Some(message),
                };
                if let Some(message) = message {
                    self.errors.borrow_mut().push(message);
                }
                Merged::Conflict(project.to_vec())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plan::Action;
    use crate::record::Inventory;

    fn plan(actions: Vec<Action>) -> Plan {
        Plan {
            actions,
            conflicts: vec![],
            inventory: Inventory::default(),
        }
    }

    fn write_file(path: &str, bytes: &[u8], executable: bool) -> Action {
        Action::Write {
            path: path.to_string(),
            entry: Entry::File {
                bytes: bytes.to_vec(),
                executable,
            },
        }
    }

    #[cfg(unix)]
    #[test]
    fn apply_writes_modes_and_symlinks() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let p = plan(vec![
            write_file("a/b/run.sh", b"#!/bin/sh\r\n", true),
            write_file("plain.txt", b"x", false),
            Action::Write {
                path: "link".to_string(),
                entry: Entry::Symlink {
                    target: "plain.txt".to_string(),
                },
            },
        ]);
        apply(&p, root).unwrap();
        let m = std::fs::metadata(root.join("a/b/run.sh")).unwrap();
        assert_eq!(m.permissions().mode() & 0o777, 0o755);
        assert_eq!(
            std::fs::read(root.join("a/b/run.sh")).unwrap(),
            b"#!/bin/sh\r\n"
        );
        let m = std::fs::metadata(root.join("plain.txt")).unwrap();
        assert_eq!(m.permissions().mode() & 0o777, 0o644);
        assert_eq!(
            std::fs::read_link(root.join("link"))
                .unwrap()
                .to_str()
                .unwrap(),
            "plain.txt"
        );
    }

    #[cfg(unix)]
    #[test]
    fn write_keeps_existing_mode_bits() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let mode_of = |p: &str| {
            std::fs::metadata(root.join(p))
                .unwrap()
                .permissions()
                .mode()
                & 0o7777
        };
        for (start, executable, expected) in [
            (0o600, false, 0o600),
            (0o600, true, 0o700),
            (0o700, false, 0o600),
            (0o640, true, 0o750),
            (0o644, true, 0o755),
            (0o755, false, 0o644),
        ] {
            std::fs::write(root.join("f"), "old").unwrap();
            std::fs::set_permissions(root.join("f"), std::fs::Permissions::from_mode(start))
                .unwrap();
            apply(&plan(vec![write_file("f", b"new", executable)]), root).unwrap();
            assert_eq!(mode_of("f"), expected, "{start:o} exec={executable}");
            assert_eq!(std::fs::read(root.join("f")).unwrap(), b"new");
        }
    }

    #[cfg(unix)]
    #[test]
    fn write_drops_setuid_bits() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        for (start, executable, expected) in [
            (0o4755, true, 0o755),
            (0o2755, true, 0o755),
            (0o1644, false, 0o644),
        ] {
            std::fs::write(root.join("f"), "old").unwrap();
            std::fs::set_permissions(root.join("f"), std::fs::Permissions::from_mode(start))
                .unwrap();
            apply(&plan(vec![write_file("f", b"new", executable)]), root).unwrap();
            let mode = std::fs::metadata(root.join("f"))
                .unwrap()
                .permissions()
                .mode()
                & 0o7777;
            assert_eq!(mode, expected, "{start:o} exec={executable}");
        }
    }

    #[test]
    fn delete_prunes_empty_dirs() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("a/b")).unwrap();
        std::fs::create_dir_all(root.join("c/d")).unwrap();
        std::fs::write(root.join("a/b/f"), "1").unwrap();
        std::fs::write(root.join("c/d/f"), "1").unwrap();
        std::fs::write(root.join("c/keep"), "1").unwrap();
        let p = plan(vec![
            Action::Delete {
                path: "a/b/f".into(),
            },
            Action::Delete {
                path: "c/d/f".into(),
            },
        ]);
        apply(&p, root).unwrap();
        assert!(!root.join("a").exists());
        assert!(!root.join("c/d").exists());
        assert!(root.join("c/keep").exists());
        assert!(root.exists());
    }

    #[cfg(unix)]
    #[test]
    fn disk_view_reads_symlink_as_symlink() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::write(root.join("f"), "data").unwrap();
        std::fs::set_permissions(root.join("f"), std::fs::Permissions::from_mode(0o755)).unwrap();
        std::os::unix::fs::symlink("f", root.join("l")).unwrap();
        std::fs::create_dir(root.join(".git")).unwrap();
        std::fs::write(root.join(".git/x"), "1").unwrap();
        let view = DiskProject::new(root);
        assert_eq!(
            view.get("l"),
            Some(Entry::Symlink {
                target: "f".to_string()
            })
        );
        assert_eq!(
            view.get("f"),
            Some(Entry::File {
                bytes: b"data".to_vec(),
                executable: true
            })
        );
        assert_eq!(view.get("missing"), None);
        assert_eq!(view.get(".git/x"), None);
        assert_eq!(view.get(".git"), None);
        view.finish().unwrap();
    }

    #[test]
    fn disk_view_directory_is_absent() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("sub")).unwrap();
        let view = DiskProject::new(dir.path());
        assert_eq!(view.get("sub"), None);
        view.finish().unwrap();
    }

    #[test]
    fn disk_view_file_parent_is_absent() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("f"), "x").unwrap();
        let view = DiskProject::new(dir.path());
        assert_eq!(view.get("f/x"), None);
        assert_eq!(view.get("f/x/y"), None);
        view.finish().unwrap();
    }

    #[test]
    fn disk_view_lists_files() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("d/e/empty")).unwrap();
        std::fs::write(root.join("d/a"), "1").unwrap();
        std::fs::write(root.join("d/e/b"), "2").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink("a", root.join("d/l")).unwrap();
        let view = DiskProject::new(root);
        let mut files = view.list_files("d").unwrap();
        files.sort();
        let mut want = vec!["d/a", "d/e/b", "d/e/empty"];
        if cfg!(unix) {
            want.push("d/l");
        }
        assert_eq!(files, want);
        assert_eq!(view.list_files("d/e/empty"), Some(vec![]));
        assert_eq!(view.list_files("d/a"), None);
        assert_eq!(view.list_files("missing"), None);
        assert_eq!(view.list_files("d/a/x"), None);
        view.finish().unwrap();
    }

    #[test]
    fn disk_view_lists_empty_dirs() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("docs/empty")).unwrap();
        std::fs::create_dir_all(root.join("docs/a/b")).unwrap();
        std::fs::write(root.join("docs/x.txt"), "x").unwrap();
        let view = DiskProject::new(root);
        let mut files = view.list_files("docs").unwrap();
        files.sort();
        assert_eq!(files, ["docs/a/b", "docs/empty", "docs/x.txt"]);
        view.finish().unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn disk_view_list_files_follows_no_links() {
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("f"), "secret").unwrap();
        let dir = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(outside.path(), dir.path().join("s")).unwrap();
        let view = DiskProject::new(dir.path());
        assert_eq!(view.list_files("s"), None);
        view.finish().unwrap();
        let view = DiskProject::new(dir.path());
        assert_eq!(view.list_files("s/x"), None);
        let err = view.finish().unwrap_err().to_string();
        assert!(err.contains("symlink"), "{err}");
    }

    #[test]
    fn apply_replaces_emptied_directory() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("docs")).unwrap();
        apply(&plan(vec![write_file("docs", b"d", false)]), dir.path()).unwrap();
        assert_eq!(std::fs::read(dir.path().join("docs")).unwrap(), b"d");
    }

    #[cfg(unix)]
    #[test]
    fn apply_refuses_symlinked_parent() {
        let outside = tempfile::tempdir().unwrap();
        let dir = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(outside.path(), dir.path().join("d")).unwrap();
        let err = apply(&plan(vec![write_file("d/f", b"x", false)]), dir.path()).unwrap_err();
        assert!(err.to_string().contains("symlink"), "{err}");
        assert_eq!(std::fs::read_dir(outside.path()).unwrap().count(), 0);
    }

    #[cfg(unix)]
    #[test]
    fn apply_replaces_leaf_symlink_without_following() {
        let outside = tempfile::tempdir().unwrap();
        let target = outside.path().join("victim");
        std::fs::write(&target, "original").unwrap();
        let dir = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(&target, dir.path().join("f")).unwrap();
        apply(&plan(vec![write_file("f", b"new", false)]), dir.path()).unwrap();
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "original");
        let meta = std::fs::symlink_metadata(dir.path().join("f")).unwrap();
        assert!(meta.is_file());
        assert_eq!(std::fs::read(dir.path().join("f")).unwrap(), b"new");
    }

    #[test]
    fn apply_rejects_unsafe_paths() {
        let outer = tempfile::tempdir().unwrap();
        let root = outer.path().join("root");
        std::fs::create_dir(&root).unwrap();
        for bad in ["../x", "/x", "a//b", "a/./b"] {
            let err = apply(&plan(vec![write_file(bad, b"x", false)]), &root);
            assert!(err.is_err(), "{bad} accepted");
            let err = apply(&plan(vec![Action::Delete { path: bad.into() }]), &root);
            assert!(err.is_err(), "delete {bad} accepted");
        }
        assert!(!outer.path().join("x").exists());
        assert!(!std::path::Path::new("/x").exists());
        assert_eq!(std::fs::read_dir(&root).unwrap().count(), 0);
        let view = DiskProject::new(&root);
        assert_eq!(view.get("../x"), None);
        assert!(view.finish().is_err());
    }

    #[test]
    fn check_path_rejects_backslash_and_colon() {
        for bad in ["a\\b", "\\x", "a/b\\c", "c:x", "a/b:c", "C:"] {
            assert!(check_path(bad).is_err(), "{bad} accepted");
        }
        for good in ["a", "a/b.c", "a-b/c_d"] {
            check_path(good).unwrap();
        }
    }

    #[cfg(unix)]
    #[test]
    fn disk_view_records_symlinked_parent() {
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("f"), "secret").unwrap();
        let dir = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(outside.path(), dir.path().join("d")).unwrap();
        let view = DiskProject::new(dir.path());
        assert_eq!(view.get("d/f"), None);
        let err = view.finish().unwrap_err().to_string();
        assert!(err.contains("path goes through symlink"), "{err}");
        assert!(err.contains('d'), "{err}");
    }

    #[test]
    fn delete_missing_leaf_keeps_empty_dirs() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("a/b")).unwrap();
        apply(
            &plan(vec![Action::Delete {
                path: "a/b/gone".into(),
            }]),
            dir.path(),
        )
        .unwrap();
        assert!(dir.path().join("a/b").is_dir());
    }

    #[cfg(unix)]
    #[test]
    fn delete_symlink_keeps_target() {
        let outside = tempfile::tempdir().unwrap();
        let target = outside.path().join("victim");
        std::fs::write(&target, "keep").unwrap();
        let dir = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(&target, dir.path().join("l")).unwrap();
        apply(&plan(vec![Action::Delete { path: "l".into() }]), dir.path()).unwrap();
        assert!(std::fs::symlink_metadata(dir.path().join("l")).is_err());
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "keep");
    }

    #[cfg(unix)]
    #[test]
    fn delete_through_symlinked_parent_errors() {
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("f"), "keep").unwrap();
        let dir = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(outside.path(), dir.path().join("d")).unwrap();
        let err = apply(
            &plan(vec![Action::Delete { path: "d/f".into() }]),
            dir.path(),
        );
        assert!(err.is_err());
        assert!(outside.path().join("f").exists());
    }

    #[cfg(unix)]
    #[test]
    fn symlink_write_replaces_existing_file() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("f"), "old").unwrap();
        let p = plan(vec![Action::Write {
            path: "f".into(),
            entry: Entry::Symlink {
                target: "elsewhere".into(),
            },
        }]);
        apply(&p, dir.path()).unwrap();
        assert_eq!(
            std::fs::read_link(dir.path().join("f")).unwrap().to_str(),
            Some("elsewhere")
        );
    }

    #[test]
    fn apply_failure_reports_progress() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("dir")).unwrap();
        std::fs::write(dir.path().join("dir/kept"), "k").unwrap();
        let p = plan(vec![
            write_file("ok", b"1", false),
            write_file("dir", b"2", false),
        ]);
        let err = apply(&p, dir.path()).unwrap_err().to_string();
        assert!(err.contains("applied 1 of 2"), "{err}");
        assert!(err.contains("dir"), "{err}");
    }

    fn lines(n: usize, edits: &[(usize, &str)]) -> String {
        (1..=n)
            .map(|i| {
                let text = edits
                    .iter()
                    .find(|(k, _)| *k == i)
                    .map_or(format!("line{i}"), |(_, t)| (*t).to_string());
                format!("{text}\n")
            })
            .collect()
    }

    fn merged(base: &[u8], project: &[u8], target: &[u8]) -> Merged {
        let merge = GitMerge::new();
        let out = merge.merge3("f.txt", base, project, target, "t");
        merge.finish().unwrap();
        out
    }

    fn conflict(m: Merged) -> String {
        match m {
            Merged::Conflict(b) => String::from_utf8(b).unwrap(),
            Merged::Clean(b) => panic!("expected conflict, got clean {b:?}"),
        }
    }

    #[test]
    fn non_overlapping_edits_merge_clean() {
        let base = lines(10, &[]);
        let project = lines(10, &[(2, "mine")]);
        let target = lines(10, &[(9, "theirs")]);
        let want = lines(10, &[(2, "mine"), (9, "theirs")]);
        assert_eq!(
            merged(base.as_bytes(), project.as_bytes(), target.as_bytes()),
            Merged::Clean(want.into_bytes())
        );
    }

    #[test]
    fn overlap_gives_markers() {
        let merge = GitMerge::new();
        let out = conflict(merge.merge3(
            "f.txt",
            b"a\nb\nc\n",
            b"a\nmine\nc\n",
            b"a\ntheirs\nc\n",
            "template v1.1.0",
        ));
        merge.finish().unwrap();
        assert_eq!(
            out,
            "a\n<<<<<<< project\nmine\n=======\ntheirs\n>>>>>>> template v1.1.0\nc\n"
        );
    }

    #[test]
    fn empty_base_identical_is_clean() {
        assert_eq!(merged(b"", b"x\n", b"x\n"), Merged::Clean(b"x\n".to_vec()));
    }

    #[test]
    fn empty_base_different_conflicts() {
        let out = conflict(merged(b"", b"x\n", b"y\n"));
        assert_eq!(out, "<<<<<<< project\nx\n=======\ny\n>>>>>>> t\n");
    }

    #[test]
    fn crlf_preserved() {
        let clean = merged(
            b"a\r\nb\r\nc\r\nd\r\n",
            b"A\r\nb\r\nc\r\nd\r\n",
            b"a\r\nb\r\nc\r\nD\r\n",
        );
        assert_eq!(clean, Merged::Clean(b"A\r\nb\r\nc\r\nD\r\n".to_vec()));
        let out = conflict(merged(b"a\r\nb\r\n", b"a\r\nx\r\n", b"a\r\ny\r\n"));
        assert_eq!(
            out,
            "a\r\n<<<<<<< project\r\nx\r\n=======\r\ny\r\n>>>>>>> t\r\n"
        );
    }

    #[test]
    fn no_final_newline_preserved() {
        let clean = merged(b"a\nb\nc", b"A\nb\nc", b"a\nb\nC");
        assert_eq!(clean, Merged::Clean(b"A\nb\nC".to_vec()));
        let out = conflict(merged(b"a\nb", b"a\nx", b"a\ny"));
        assert_eq!(out, "a\n<<<<<<< project\nx\n=======\ny\n>>>>>>> t\n");
    }

    #[test]
    fn crlf_incomplete_side_gets_crlf() {
        let out = conflict(merged(b"a\r\nb", b"a\r\nx", b"a\r\ny"));
        assert_eq!(
            out,
            "a\r\n<<<<<<< project\r\nx\r\n=======\r\ny\r\n>>>>>>> t\r\n"
        );
        // A side that ends with its own line break keeps it as is.
        let out = conflict(merged(b"a\r\nb\r\n", b"a\r\nx\r\n", b"a\r\ny"));
        assert_eq!(
            out,
            "a\r\n<<<<<<< project\r\nx\r\n=======\r\ny\r\n>>>>>>> t\r\n"
        );
    }

    #[test]
    fn marker_lookalike_content_untouched() {
        let base = lines(10, &[]);
        let project = lines(10, &[(2, "<<<<<<< x")]);
        let target = lines(10, &[(9, "theirs")]);
        let want = lines(10, &[(2, "<<<<<<< x"), (9, "theirs")]);
        assert_eq!(
            merged(base.as_bytes(), project.as_bytes(), target.as_bytes()),
            Merged::Clean(want.into_bytes())
        );
        let project = "a\n<<<<<<< x\n=======\nc\n";
        let out = conflict(merged(b"a\nb\nc\n", project.as_bytes(), b"a\nq\nc\n"));
        assert_eq!(out.matches("<<<<<<< project\n").count(), 1);
        assert_eq!(out.matches(">>>>>>> t\n").count(), 1);
        assert!(out.contains("<<<<<<< x\n"));
    }

    #[test]
    fn same_edit_at_the_edge_conflicts() {
        let out = merged(
            b"  \"deny\": [\n    \"a\",\n    \"b\"\n  ]\n",
            b"  \"deny\": [\n    \"a\",\n    \"b\",\n    \"p1\",\n    \"p2\"\n  ]\n",
            b"  \"deny\": [\n    \"a\",\n    \"b\",\n    \"t1\"\n  ]\n",
        );
        assert_eq!(
            out,
            Merged::Conflict(
                b"  \"deny\": [\n    \"a\",\n    \"b\",\n<<<<<<< project\n    \"p1\",\n    \"p2\"\n=======\n    \"t1\"\n>>>>>>> t\n  ]\n".to_vec()
            )
        );
    }

    #[test]
    fn conflicts_join_across_a_line_without_letters() {
        assert_eq!(
            merged(b"a\n}\nb\n", b"A\n}\nB\n", b"X\n}\nY\n"),
            Merged::Conflict(b"<<<<<<< project\nA\n}\nB\n=======\nX\n}\nY\n>>>>>>> t\n".to_vec())
        );
    }

    #[test]
    fn conflicts_stay_apart_over_four_lettered_lines() {
        assert_eq!(
            merged(
                b"a\nk1\nk2\nk3\nk4\nb\n",
                b"A\nk1\nk2\nk3\nk4\nB\n",
                b"X\nk1\nk2\nk3\nk4\nY\n"
            ),
            Merged::Conflict(
                b"<<<<<<< project\nA\n=======\nX\n>>>>>>> t\nk1\nk2\nk3\nk4\n<<<<<<< project\nB\n=======\nY\n>>>>>>> t\n".to_vec()
            )
        );
    }

    #[test]
    fn git_merges_clean_where_diffy_conflicted() {
        assert_eq!(
            merged(b"d\n\nx\n", b"x\nx\n", b"d\n\nx\n  e\n  e,\n"),
            Merged::Clean(b"x\nx\n  e\n  e,\n".to_vec())
        );
        assert_eq!(
            merged(
                b"}\na\n}\n}\nb\n",
                b"}\na\nc\n}\nb\n",
                b"}\na\n}\n}\nd\nb\n"
            ),
            Merged::Clean(b"}\na\nc\n}\nd\nb\n".to_vec())
        );
    }

    #[test]
    fn git_conflicts_where_diffy_merged_clean() {
        assert_eq!(
            merged(
                b"c\nf\n  e,\nf\n  e,\nx\nx\n",
                b"c\nf\n  e,\nf\n  e,\nb\nx\n",
                b"c\nf\n  e,\nf\n  e,\nx\n"
            ),
            Merged::Conflict(
                b"c\nf\n  e,\nf\n  e,\n<<<<<<< project\nb\n=======\n>>>>>>> t\nx\n".to_vec()
            )
        );
    }

    #[test]
    fn short_paths_do_not_call_git() {
        let merge = GitMerge::with_program("rootstock-no-such-program");
        assert_eq!(
            merge.merge3("f.txt", b"a\n", b"p\n", b"p\n", "t"),
            Merged::Clean(b"p\n".to_vec())
        );
        assert_eq!(
            merge.merge3("f.txt", b"a\n", b"a\n", b"t\n", "t"),
            Merged::Clean(b"t\n".to_vec())
        );
        assert_eq!(
            merge.merge3("f.txt", b"a\n", b"p\n", b"a\n", "t"),
            Merged::Clean(b"p\n".to_vec())
        );
        merge.finish().unwrap();
    }

    #[test]
    fn missing_git_is_reported_by_finish() {
        let merge = GitMerge::with_program("rootstock-no-such-program");
        let out = merge.merge3("dir/f.txt", b"a\n", b"p\n", b"t\n", "t");
        assert_eq!(out, Merged::Conflict(b"p\n".to_vec()));
        let err = merge.finish().unwrap_err().to_string();
        // A spawn failure is reported once, for the first file.
        assert!(err.contains("`dir/f.txt`"), "{err}");
        assert!(err.contains("failed to run"), "{err}");
        assert_eq!(err.matches("failed to run").count(), 1, "{err}");
    }

    #[cfg(unix)]
    #[test]
    fn a_failing_git_exit_is_reported_with_the_path() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("fake-git");
        std::fs::write(&script, "#!/bin/sh\necho boom >&2\nexit 255\n").unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        let merge = GitMerge::with_program(&script.to_string_lossy());
        let out = merge.merge3("dir/f.txt", b"a\n", b"p\n", b"t\n", "t");
        assert_eq!(out, Merged::Conflict(b"p\n".to_vec()));
        let err = merge.finish().unwrap_err().to_string();
        assert!(err.contains("`dir/f.txt`"), "{err}");
        assert!(err.contains("exit 255"), "{err}");
        assert!(err.contains("boom"), "{err}");
    }

    fn link_write(path: &str) -> Action {
        Action::Write {
            path: path.to_string(),
            entry: Entry::Symlink {
                target: "t".to_string(),
            },
        }
    }

    #[test]
    fn symlink_refusal_names_the_remedy() {
        let error = std::io::Error::from(std::io::ErrorKind::PermissionDenied);
        let windows = symlink_refusal(true, &error).to_string();
        assert!(
            windows.starts_with(
                "cannot create symlinks: enable Developer Mode in Windows settings, or run as administrator"
            ),
            "{windows}"
        );
        let other = symlink_refusal(false, &error).to_string();
        assert!(
            other.starts_with("cannot create symlinks on this file system"),
            "{other}"
        );
        assert!(!other.contains("Developer Mode"), "{other}");
    }

    #[test]
    fn a_link_follows_the_links_its_target_runs_through() {
        let link = |path: &str, target: &str| Action::Write {
            path: path.to_string(),
            entry: Entry::Symlink {
                target: target.to_string(),
            },
        };
        let actions = vec![
            link("a/first", "../second"),
            link("second", "dir/real.txt"),
            link("third", "viaDir/x"),
            link("viaDir", "dir"),
            write_file("dir/real.txt", b"x", false),
        ];
        let paths = |order: Vec<&Action>| -> Vec<String> {
            order
                .into_iter()
                .map(|a| match a {
                    Action::Write { path, .. } | Action::Delete { path } => path.clone(),
                })
                .collect()
        };
        assert_eq!(
            paths(ordered(&actions, true)),
            ["dir/real.txt", "second", "a/first", "viaDir", "third"]
        );
        // Cycles do not hang: the links keep their order.
        let cycle = vec![link("x", "y"), link("y", "x")];
        assert_eq!(paths(ordered(&cycle, true)), ["x", "y"]);
    }

    #[test]
    fn resolved_target_is_lexical_and_stays_in_the_project() {
        assert_eq!(
            resolved_target("a/b/l", "../c/./d").as_deref(),
            Some("a/c/d")
        );
        assert_eq!(resolved_target("l", "sub\\f").as_deref(), Some("sub/f"));
        assert_eq!(resolved_target("l", "../out"), None);
    }

    #[test]
    fn links_go_last_only_when_asked() {
        let actions = vec![
            link_write("l1"),
            write_file("a", b"x", false),
            Action::Delete { path: "d".into() },
            link_write("l2"),
        ];
        let paths = |order: Vec<&Action>| -> Vec<String> {
            order
                .into_iter()
                .map(|a| match a {
                    Action::Write { path, .. } | Action::Delete { path } => path.clone(),
                })
                .collect()
        };
        assert_eq!(paths(ordered(&actions, false)), ["l1", "a", "d", "l2"]);
        assert_eq!(paths(ordered(&actions, true)), ["a", "d", "l1", "l2"]);
    }

    #[test]
    fn link_kind_follows_the_target_on_disk() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("d")).unwrap();
        std::fs::write(dir.path().join("f"), "x").unwrap();
        assert!(link_is_dir(dir.path(), "d"));
        assert!(!link_is_dir(dir.path(), "f"));
        assert!(!link_is_dir(dir.path(), "missing"));
    }

    #[test]
    fn probe_leaves_nothing_behind() {
        let dir = tempfile::tempdir().unwrap();
        // Links are allowed where this test runs (CI runners may link as administrator).
        let with_link = plan(vec![link_write("l")]);
        check_symlinks_allowed(&with_link, dir.path()).unwrap();
        // The project directory is not made yet: the probe uses its nearest ancestor.
        let missing = dir.path().join("not/yet/there");
        check_symlinks_allowed(&with_link, &missing).unwrap();
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
        // A plan without links probes nothing, even under a path that cannot exist.
        let no_link = plan(vec![write_file("a", b"x", false)]);
        check_symlinks_allowed(&no_link, Path::new("/proc/none/at/all")).unwrap();
    }

    fn entry(kind: IndexKind, path: &str) -> IndexEntry {
        IndexEntry {
            kind,
            sha: format!("sha-of-{path}"),
            stage: 0,
            path: path.to_string(),
        }
    }

    #[test]
    fn index_ops_compare_with_the_index() {
        let index = [
            entry(IndexKind::File, "plain"),
            entry(IndexKind::Executable, "run"),
            entry(IndexKind::Executable, "same"),
            entry(IndexKind::Symlink, "link"),
            entry(IndexKind::Symlink, "link-to-exec"),
            entry(IndexKind::File, "kept"),
        ];
        let files = [
            ("plain", true),
            ("run", false),
            ("same", true),
            ("link", false),
            ("link-to-exec", true),
            ("kept", false),
            ("new.sh", true),
            ("new.txt", false),
        ];
        let ops = plan_index_ops(&files, &index, &|_| true, &|_| false);
        assert_eq!(
            ops,
            [
                IndexOp::SetMode {
                    path: "plain".to_string(),
                    mode: MODE_EXECUTABLE,
                    sha: "sha-of-plain".to_string()
                },
                IndexOp::SetMode {
                    path: "run".to_string(),
                    mode: MODE_FILE,
                    sha: "sha-of-run".to_string()
                },
                IndexOp::ReplaceLink {
                    path: "link".to_string(),
                    mode: MODE_FILE
                },
                IndexOp::ReplaceLink {
                    path: "link-to-exec".to_string(),
                    mode: MODE_EXECUTABLE
                },
                IndexOp::AddExecutable {
                    path: "new.sh".to_string()
                },
            ]
        );
    }

    #[test]
    fn index_ops_skip_missing_ignored_submodule_and_unmerged_paths() {
        let mut unmerged = entry(IndexKind::File, "conflicted");
        unmerged.stage = 2;
        let index = [
            entry(IndexKind::Gitlink, "vendor/lib"),
            entry(IndexKind::File, "gone"),
            entry(IndexKind::File, "ignored"),
            unmerged,
        ];
        let files = [
            ("vendor/lib/run.sh", true),
            ("gone", true),
            ("ignored", true),
            ("ignored-new.sh", true),
            ("conflicted", true),
            // `vendor/library` only shares a name prefix with the submodule.
            ("vendor/library/run.sh", true),
        ];
        let ops = plan_index_ops(&files, &index, &|path| path != "gone", &|path| {
            path.starts_with("ignored")
        });
        assert_eq!(
            ops,
            [IndexOp::AddExecutable {
                path: "vendor/library/run.sh".to_string()
            }]
        );
    }

    #[test]
    fn parse_index_reads_modes_stages_and_odd_names() {
        let listing = b"100755 aaaa 0\trun.sh\x00120000 bbbb 0\tdir name/l\x00\
160000 cccc 0\tvendor\x00100644 dddd 2\tx[1].sh\x00junk\x00";
        let entries = parse_index(listing);
        let summary: Vec<_> = entries
            .iter()
            .map(|e| (e.kind, e.sha.as_str(), e.stage, e.path.as_str()))
            .collect();
        assert_eq!(
            summary,
            [
                (IndexKind::Executable, "aaaa", 0, "run.sh"),
                (IndexKind::Symlink, "bbbb", 0, "dir name/l"),
                (IndexKind::Gitlink, "cccc", 0, "vendor"),
                (IndexKind::File, "dddd", 2, "x[1].sh"),
            ]
        );
    }

    /// A repository in a temp dir, with `files` committed and the bit of `exec` off.
    fn repo_with(files: &[&str]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_string_lossy().into_owned();
        crate::source::git(&["-C", &root, "init", "-q"]).unwrap();
        for file in files {
            let full = dir.path().join(file);
            std::fs::create_dir_all(full.parent().unwrap()).unwrap();
            std::fs::write(&full, format!("content of {file}\n")).unwrap();
        }
        crate::source::git(&["-C", &root, "add", "-A"]).unwrap();
        dir
    }

    /// The index mode of every path under `dir`, by path.
    fn index_modes(dir: &Path) -> std::collections::BTreeMap<String, String> {
        let out = crate::source::git(&["-C", &dir.to_string_lossy(), "ls-files", "-s"]).unwrap();
        out.lines()
            .filter_map(|line| {
                let (meta, path) = line.split_once('\t')?;
                Some((path.to_string(), meta.split(' ').next()?.to_string()))
            })
            .collect()
    }

    #[test]
    fn set_index_modes_changes_only_the_mode_of_tracked_files() {
        let repo = repo_with(&["run.sh", "keep.sh"]);
        let root = repo.path();
        let blob_before =
            crate::source::git(&["-C", &root.to_string_lossy(), "rev-parse", ":run.sh"]).unwrap();
        // The content on disk differs from the index: the index content must stay.
        std::fs::write(root.join("run.sh"), "edited\n").unwrap();
        let p = plan(vec![
            write_file("run.sh", b"edited\n", true),
            write_file("keep.sh", b"content of keep.sh\n", false),
        ]);
        assert_eq!(set_index_modes_in(&p, root).unwrap(), Vec::<String>::new());
        let modes = index_modes(root);
        assert_eq!(modes["run.sh"], "100755");
        assert_eq!(modes["keep.sh"], "100644");
        let blob_after =
            crate::source::git(&["-C", &root.to_string_lossy(), "rev-parse", ":run.sh"]).unwrap();
        assert_eq!(blob_before, blob_after);
    }

    #[test]
    fn set_index_modes_adds_new_executables_and_leaves_plain_files() {
        let repo = repo_with(&["tracked.txt"]);
        let root = repo.path();
        std::fs::write(root.join("new.sh"), "x").unwrap();
        std::fs::write(root.join("new.txt"), "x").unwrap();
        let p = plan(vec![
            write_file("new.sh", b"x", true),
            write_file("new.txt", b"x", false),
        ]);
        assert!(set_index_modes_in(&p, root).unwrap().is_empty());
        let modes = index_modes(root);
        assert_eq!(modes["new.sh"], "100755");
        assert!(!modes.contains_key("new.txt"));
    }

    #[test]
    fn set_index_modes_takes_names_literally() {
        let repo = repo_with(&["x1.sh", "x[1].sh", "other.sh"]);
        let root = repo.path();
        // An untracked file named like a glob must not stage its neighbours.
        std::fs::write(root.join("b.sh"), "b").unwrap();
        std::fs::write(root.join("*.sh"), "star").unwrap();
        let p = plan(vec![
            write_file("x[1].sh", b"content of x[1].sh\n", true),
            write_file("*.sh", b"star", true),
        ]);
        assert!(set_index_modes_in(&p, root).unwrap().is_empty());
        let modes = index_modes(root);
        assert_eq!(modes["x[1].sh"], "100755");
        assert_eq!(modes["x1.sh"], "100644");
        assert_eq!(modes["other.sh"], "100644");
        assert_eq!(modes["*.sh"], "100755");
        assert!(!modes.contains_key("b.sh"));
    }

    #[test]
    fn set_index_modes_skips_ignored_paths() {
        let repo = repo_with(&[]);
        let root = repo.path();
        std::fs::write(root.join(".gitignore"), "ignored.sh\n").unwrap();
        std::fs::write(root.join("ignored.sh"), "x").unwrap();
        let p = plan(vec![write_file("ignored.sh", b"x", true)]);
        assert!(set_index_modes_in(&p, root).unwrap().is_empty());
        assert!(!index_modes(root).contains_key("ignored.sh"));
    }

    #[test]
    fn set_index_modes_replaces_a_tracked_link_with_the_file() {
        let repo = repo_with(&[]);
        let root = repo.path();
        let dir = root.to_string_lossy().into_owned();
        // A link recorded only in the index, then a file written over its path.
        let sha = {
            std::fs::write(root.join("target-blob"), "real.txt").unwrap();
            crate::source::git(&["-C", &dir, "hash-object", "-w", "target-blob"]).unwrap()
        };
        crate::source::git(&[
            "-C",
            &dir,
            "update-index",
            "--add",
            "--cacheinfo",
            "120000",
            &sha,
            "was-link",
        ])
        .unwrap();
        std::fs::write(root.join("was-link"), "now a file\n").unwrap();
        let p = plan(vec![write_file("was-link", b"now a file\n", true)]);
        assert!(set_index_modes_in(&p, root).unwrap().is_empty());
        assert_eq!(index_modes(root)["was-link"], "100755");
        let content = crate::source::git(&["-C", &dir, "show", ":was-link"]).unwrap();
        assert_eq!(content, "now a file");
    }

    #[test]
    fn set_index_modes_works_in_a_subdirectory_of_a_repository() {
        let repo = repo_with(&["pkg/run.sh", "other/run.sh"]);
        let project = repo.path().join("pkg");
        let p = plan(vec![write_file("run.sh", b"content of pkg/run.sh\n", true)]);
        assert!(set_index_modes_in(&p, &project).unwrap().is_empty());
        let modes = index_modes(repo.path());
        assert_eq!(modes["pkg/run.sh"], "100755");
        assert_eq!(modes["other/run.sh"], "100644");
        // The project's own view of the index has paths relative to it.
        assert_eq!(
            read_index(&project).unwrap()["run.sh"],
            FileMode::Executable
        );
    }

    #[test]
    fn a_failing_path_is_collected_and_the_rest_still_run() {
        let repo = repo_with(&["ok.sh"]);
        let root = repo.path();
        let sha =
            crate::source::git(&["-C", &root.to_string_lossy(), "rev-parse", ":ok.sh"]).unwrap();
        let ops = [
            IndexOp::AddExecutable {
                path: "does-not-exist.sh".to_string(),
            },
            IndexOp::SetMode {
                path: "ok.sh".to_string(),
                mode: MODE_EXECUTABLE,
                sha,
            },
        ];
        let failed = run_index_ops(root, "", &ops);
        assert_eq!(failed.len(), 1, "{failed:?}");
        assert!(failed[0].starts_with("`does-not-exist.sh`: "), "{failed:?}");
        assert_eq!(index_modes(root)["ok.sh"], "100755");
    }

    #[test]
    fn index_decides_the_bit_and_the_link_of_a_file() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("run.sh"), "x").unwrap();
        std::fs::write(dir.path().join("link"), "run.sh").unwrap();
        std::fs::write(dir.path().join("other"), "y").unwrap();
        let mut view = DiskProject::new(dir.path());
        view.index = Modes::from([
            ("run.sh".to_string(), FileMode::Executable),
            ("link".to_string(), FileMode::Symlink),
        ]);
        assert_eq!(
            view.get("run.sh"),
            Some(Entry::File {
                bytes: b"x".to_vec(),
                executable: true
            })
        );
        assert_eq!(
            view.get("link"),
            Some(Entry::Symlink {
                target: "run.sh".to_string()
            })
        );
        assert!(matches!(
            view.get("other"),
            Some(Entry::File {
                executable: false,
                ..
            })
        ));
        view.finish().unwrap();
    }

    #[test]
    fn set_index_modes_skips_a_directory_without_git() {
        let dir = tempfile::tempdir().unwrap();
        let p = plan(vec![write_file("run.sh", b"x", true)]);
        assert!(set_index_modes(&p, dir.path()).unwrap().is_empty());
        assert!(set_index_modes_in(&p, dir.path()).unwrap().is_empty());
    }

    /// Where the file system has no bit (not Unix), the bit goes to the index.
    #[cfg(not(unix))]
    #[test]
    fn apply_records_the_bit_in_the_index() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        crate::source::git(&["-C", &root.to_string_lossy(), "init", "-q"]).unwrap();
        crate::source::git(&["-C", &root.to_string_lossy(), "add", "--", "."]).unwrap();
        let p = plan(vec![
            write_file("run.sh", b"#!/bin/sh\n", true),
            write_file("plain.txt", b"x", false),
        ]);
        apply(&p, root).unwrap();
        assert!(set_index_modes(&p, root).unwrap().is_empty());
        let index = read_index(root).unwrap();
        assert_eq!(index.get("run.sh"), Some(&FileMode::Executable));
        assert_eq!(index.get("plain.txt"), None);
        assert!(
            DiskProject::new(root)
                .get("run.sh")
                .is_some_and(|e| matches!(
                    e,
                    Entry::File {
                        executable: true,
                        ..
                    }
                ))
        );
    }
}
