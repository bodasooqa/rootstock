//! Executes a `Plan` on disk and reads the project back as a `ProjectView`.

use std::cell::{Cell, RefCell};
use std::path::Path;
use std::process::{Command, Stdio};

use crate::RelPath;
use crate::merge::{Merge3, Merged};
use crate::plan::{Action, Plan, ProjectView};
use crate::render::Entry;

/// The project directory as a `ProjectView`; `.git` is invisible.
pub struct DiskProject<'a> {
    pub root: &'a Path,
    errors: RefCell<Vec<String>>,
}

impl<'a> DiskProject<'a> {
    pub fn new(root: &'a Path) -> Self {
        Self {
            root,
            errors: RefCell::new(Vec::new()),
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
            return Ok(Some(Entry::Symlink { target }));
        }
        if !file_type.is_file() {
            return Err(format!("`{path}` is a special file, expected a file"));
        }
        let bytes = std::fs::read(&full).map_err(|e| format!("`{path}`: {e}"))?;
        Ok(Some(Entry::File {
            bytes,
            executable: is_executable(&meta),
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
    for (done, action) in plan.actions.iter().enumerate() {
        let path = match action {
            Action::Write { path, .. } | Action::Delete { path } => path,
        };
        apply_one(action, root).map_err(|e| {
            anyhow::anyhow!("applied {done} of {total} changes; failed at `{path}`: {e:#}")
        })?;
    }
    Ok(())
}

fn apply_one(action: &Action, root: &Path) -> anyhow::Result<()> {
    match action {
        Action::Write { path, entry } => {
            check_path(path)?;
            let full = root.join(path);
            let parent = create_parents(root, path)?;
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
                Ok(_) => {
                    std::fs::remove_file(&full)?;
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

#[cfg(unix)]
fn write_symlink(parent: &Path, full: &Path, target: &str) -> anyhow::Result<()> {
    // The link is built inside a fresh temp dir, then renamed over the path.
    let scratch = tempfile::Builder::new().tempdir_in(parent)?;
    let link = scratch.path().join("link");
    std::os::unix::fs::symlink(target, &link)?;
    std::fs::rename(&link, full)?;
    Ok(())
}

#[cfg(not(unix))]
fn write_symlink(_parent: &Path, _full: &Path, _target: &str) -> anyhow::Result<()> {
    anyhow::bail!("symlinks are not supported on this platform")
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
        let mut cmd = Command::new(&self.program);
        for var in crate::source::GIT_LOCAL_ENV {
            cmd.env_remove(var);
        }
        let out = cmd
            .current_dir(dir.path())
            .args(["-c", "merge.conflictStyle=merge"])
            .args(["merge-file", "-p", "-L", "project", "-L", "base", "-L"])
            .arg(target_label)
            .args(["project", "base", "target"])
            .env("GIT_CONFIG_GLOBAL", dir.path().join("gitconfig"))
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .stdin(Stdio::null())
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
}
