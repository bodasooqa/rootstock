//! Executes a `Plan` on disk and reads the project back as a `ProjectView`.

use std::cell::RefCell;
use std::path::Path;

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

    fn read(&self, path: &str) -> Result<Option<Entry>, String> {
        let full = self.root.join(path);
        match existing_parents(self.root, path, "path goes through symlink") {
            Ok(Some(())) => {}
            Ok(None) => return Ok(None),
            Err(e) => return Err(format!("`{path}`: {e}")),
        }
        let meta = match std::fs::symlink_metadata(&full) {
            Ok(meta) => meta,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(format!("`{path}`: {e}")),
        };
        let file_type = meta.file_type();
        if file_type.is_symlink() {
            let target = std::fs::read_link(&full).map_err(|e| format!("`{path}`: {e}"))?;
            let target = target
                .into_os_string()
                .into_string()
                .map_err(|_| format!("`{path}`: symlink target is not valid UTF-8"))?;
            return Ok(Some(Entry::Symlink { target }));
        }
        if !file_type.is_file() {
            return Err(format!(
                "`{path}` is a directory or special file, expected a file"
            ));
        }
        let bytes = std::fs::read(&full).map_err(|e| format!("`{path}`: {e}"))?;
        Ok(Some(Entry::File {
            bytes,
            executable: is_executable(&meta),
        }))
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

/// Walks the parent components of `path` below `root`, refusing symlinks and
/// non-directories. Missing components are created one at a time when `create`
/// is set; otherwise a missing component yields `None`.
fn walk_parents(
    root: &Path,
    path: &str,
    create: bool,
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
                anyhow::bail!("`{}` is not a directory", current.display())
            }
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                if !create {
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
    walk_parents(root, path, true, "refusing to write through symlink")?;
    Ok(parent)
}

/// Walks existing parents only; `None` means a component is missing.
pub(crate) fn existing_parents(
    root: &Path,
    path: &str,
    symlink_message: &str,
) -> anyhow::Result<Option<()>> {
    Ok(walk_parents(root, path, false, symlink_message)?.map(|_| ()))
}

/// Rejects paths that could leave the root: empty, absolute, or with `""`, `.`, `..` segments.
pub(crate) fn check_path(path: &str) -> anyhow::Result<()> {
    let bad = path.is_empty()
        || path.starts_with('/')
        || path.starts_with('\\')
        || path.split('/').any(|s| matches!(s, "" | "." | ".."));
    if bad {
        anyhow::bail!("unsafe path `{path}`");
    }
    Ok(())
}

fn write_file(parent: &Path, full: &Path, bytes: &[u8], executable: bool) -> anyhow::Result<()> {
    use std::io::Write;
    let mut tmp = tempfile::NamedTempFile::new_in(parent)?;
    tmp.write_all(bytes)?;
    set_mode(tmp.path(), executable)?;
    tmp.persist(full).map_err(|e| e.error)?;
    Ok(())
}

#[cfg(unix)]
fn set_mode(path: &Path, executable: bool) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mode = if executable { 0o755 } else { 0o644 };
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
}

#[cfg(not(unix))]
fn set_mode(_path: &Path, _executable: bool) -> std::io::Result<()> {
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
    fn disk_view_records_directory_error() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("sub")).unwrap();
        let view = DiskProject::new(dir.path());
        assert_eq!(view.get("sub"), None);
        let err = view.finish().unwrap_err().to_string();
        assert!(err.contains("sub"), "{err}");
        assert!(err.contains("directory or special file"), "{err}");
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
        let p = plan(vec![
            write_file("ok", b"1", false),
            write_file("dir", b"2", false),
        ]);
        let err = apply(&p, dir.path()).unwrap_err().to_string();
        assert!(err.contains("applied 1 of 2"), "{err}");
        assert!(err.contains("dir"), "{err}");
    }
}
