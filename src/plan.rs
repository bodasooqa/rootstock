//! The update planner: a pure function from base render, target render and
//! the project's current state to the actions and conflicts of an update.

use std::collections::{BTreeMap, BTreeSet};

use crate::RelPath;
use crate::merge::{Merged, is_binary, merge3};
use crate::record::{FileClass, Inventory};
use crate::render::{Entry, Rendered, RenderedTree, fold_path};

/// Suffix of the side file written next to a conflicting binary file.
pub const SIDE_FILE_SUFFIX: &str = ".rootstock-new";

/// Read access to the project's current files; `None` means the path is absent.
pub trait ProjectView {
    /// The file or symlink at `path`; `None` when nothing is there, when a
    /// directory is there, or when a parent component is not a directory.
    fn get(&self, path: &str) -> Option<Entry>;

    /// Everything below directory `dir` that is not a directory (files,
    /// symlinks), plus every empty directory below it, recursively, as full
    /// paths; `None` when `dir` is not a directory. An empty directory is never
    /// deleted by a plan, so it always counts as something the project keeps.
    fn list_files(&self, dir: &str) -> Option<Vec<RelPath>>;
}

/// One change to apply to the project.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Write { path: RelPath, entry: Entry },
    Delete { path: RelPath },
}

impl Action {
    fn path(&self) -> &str {
        match self {
            Action::Write { path, .. } | Action::Delete { path } => path,
        }
    }
}

/// Why a path needs a human.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConflictKind {
    Markers,
    Binary,
    Symlink,
    RemovedByTemplate,
    DeletedByProject,
    AddedByBoth,
    /// The template's path is occupied by something the project keeps.
    Blocked,
}

impl ConflictKind {
    /// Name used in text and JSON output.
    pub fn as_str(&self) -> &'static str {
        match self {
            ConflictKind::Markers => "markers",
            ConflictKind::Binary => "binary",
            ConflictKind::Symlink => "symlink",
            ConflictKind::RemovedByTemplate => "removed-by-template",
            ConflictKind::DeletedByProject => "deleted-by-project",
            ConflictKind::AddedByBoth => "added-by-both",
            ConflictKind::Blocked => "blocked",
        }
    }
}

/// A path that needs attention after the update.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Conflict {
    pub path: RelPath,
    pub kind: ConflictKind,
}

/// Result of planning: what to do, what conflicts, and the new inventory.
///
/// `actions` lists every `Delete`, sorted by path, then every `Write`, sorted
/// by path: a rename or a file replaced by a directory needs the old path gone
/// before the new one is written. `conflicts` is sorted by path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    pub actions: Vec<Action>,
    pub conflicts: Vec<Conflict>,
    pub inventory: Inventory,
}

/// Content equality: bytes for files, link target for symlinks; the executable bit is ignored.
fn same_content(a: &Entry, b: &Entry) -> bool {
    match (a, b) {
        (Entry::File { bytes: x, .. }, Entry::File { bytes: y, .. }) => x == y,
        (Entry::Symlink { target: x }, Entry::Symlink { target: y }) => x == y,
        _ => false,
    }
}

fn is_binary_entry(entry: &Entry) -> bool {
    matches!(entry, Entry::File { bytes, .. } if is_binary(bytes))
}

fn file_bytes(entry: &Entry) -> Option<&[u8]> {
    match entry {
        Entry::File { bytes, .. } => Some(bytes),
        Entry::Symlink { .. } => None,
    }
}

#[derive(Default)]
struct Out {
    actions: Vec<Action>,
    conflicts: Vec<Conflict>,
}

impl Out {
    /// Write `entry` unless the project already has exactly it.
    fn write(&mut self, path: &str, entry: Entry, project: Option<&Entry>) {
        if project != Some(&entry) {
            self.actions.push(Action::Write {
                path: path.to_string(),
                entry,
            });
        }
    }

    fn conflict(&mut self, path: &str, kind: ConflictKind) {
        self.conflicts.push(Conflict {
            path: path.to_string(),
            kind,
        });
    }

    /// Binary rule: keep the project's copy, put the target next to it.
    /// When the side file's name is itself a template path, no side write is emitted.
    fn binary_conflict(&mut self, path: &str, target: &Entry, side_taken: bool) {
        if !side_taken {
            self.actions.push(Action::Write {
                path: format!("{path}{SIDE_FILE_SUFFIX}"),
                entry: target.clone(),
            });
        }
        self.conflict(path, ConflictKind::Binary);
    }
}

/// Decides what to do with one path that is managed in the target and
/// present in the project, when the base is `base` (`None` = added by both).
fn plan_present(
    out: &mut Out,
    path: &str,
    base: Option<&Entry>,
    target: &Entry,
    project: &Entry,
    label: &str,
    side_taken: bool,
) {
    if same_content(project, target) {
        out.write(path, target.clone(), Some(project));
        return;
    }
    if let Some(b) = base {
        // The template did not change this path: the project's edits stand.
        if b == target {
            return;
        }
        if same_content(b, target) {
            // Only the executable bit changed: keep the project's content, take the bit.
            if let (Entry::File { bytes, .. }, Entry::File { executable, .. }) = (project, target) {
                let entry = Entry::File {
                    bytes: bytes.clone(),
                    executable: *executable,
                };
                out.write(path, entry, Some(project));
            }
            return;
        }
        if same_content(project, b) {
            out.write(path, target.clone(), Some(project));
            return;
        }
    }
    let involved: Vec<&Entry> = base.into_iter().chain([project, target]).collect();
    if involved.iter().any(|e| is_binary_entry(e)) {
        out.binary_conflict(path, target, side_taken);
        return;
    }
    let (Some(pb), Some(tb)) = (file_bytes(project), file_bytes(target)) else {
        out.conflict(path, ConflictKind::Symlink);
        return;
    };
    let base_bytes: &[u8] = match base {
        None => b"",
        Some(b) => match file_bytes(b) {
            Some(bytes) => bytes,
            None => {
                out.conflict(path, ConflictKind::Symlink);
                return;
            }
        },
    };
    let (merged, kind) = match merge3(base_bytes, pb, tb, label) {
        Merged::Clean(bytes) => (bytes, base.is_none().then_some(ConflictKind::AddedByBoth)),
        Merged::Conflict(bytes) => (
            bytes,
            Some(if base.is_none() {
                ConflictKind::AddedByBoth
            } else {
                ConflictKind::Markers
            }),
        ),
    };
    let executable = matches!(
        target,
        Entry::File {
            executable: true,
            ..
        }
    );
    out.write(
        path,
        Entry::File {
            bytes: merged,
            executable,
        },
        Some(project),
    );
    if let Some(kind) = kind {
        out.conflict(path, kind);
    }
}

/// True when one of the actions emitted for a single path writes that path.
fn wrote_path(actions: &[Action], path: &str) -> bool {
    actions
        .iter()
        .any(|a| matches!(a, Action::Write { path: p, .. } if p == path))
}

/// Pairs a base-only path with a target-only path of the same fold: a rename
/// that differs only in case (or in what the filesystem folds away).
/// Returns target path -> (base path, project entry at the base path).
/// Render rejects folded collisions inside one tree, so the pairing is one-to-one.
///
/// The design plans a rename as a delete plus an add. Pairing exists only
/// because on a case-insensitive filesystem both hit one file, so a pair is
/// kept only when the project shows that alias: an entry at the new name
/// equal to the one at the old name. Every other case, including nothing at
/// either name, goes through the normal rows path by path.
fn rename_pairs<'a>(
    base: &'a RenderedTree,
    target: &'a RenderedTree,
    project: &dyn ProjectView,
) -> BTreeMap<&'a RelPath, (&'a RelPath, Entry)> {
    let added: BTreeMap<String, &RelPath> = target
        .keys()
        .filter(|t| !base.contains_key(*t))
        .map(|t| (fold_path(t), t))
        .collect();
    base.keys()
        .filter(|b| !target.contains_key(*b))
        .filter_map(|b| added.get(&fold_path(b)).map(|t| (*t, b)))
        .filter_map(|(t, b)| {
            let at_new = project.get(t)?;
            (project.get(b).as_ref() == Some(&at_new)).then_some((t, (b, at_new)))
        })
        .collect()
}

/// Plans a rename pair `old` -> `new` as one logical path; `p` is the project's
/// entry at `old`, which `new` aliases on a case-insensitive filesystem.
#[allow(clippy::too_many_arguments)]
fn plan_rename(
    out: &mut Out,
    new_inventory: &mut Inventory,
    old: &str,
    new: &str,
    b: &Rendered,
    t: &Rendered,
    p: &Entry,
    label: &str,
    side_taken: bool,
) {
    match t.class {
        FileClass::Starter => {
            // A starter's content belongs to the project: only the name follows the template.
            out.actions.push(Action::Delete {
                path: old.to_string(),
            });
            out.actions.push(Action::Write {
                path: new.to_string(),
                entry: p.clone(),
            });
        }
        FileClass::Managed => {
            let mut scratch = Out::default();
            plan_present(
                &mut scratch,
                new,
                Some(&b.entry),
                &t.entry,
                p,
                label,
                side_taken,
            );
            let mut result = p.clone();
            for action in scratch.actions {
                match action {
                    Action::Write { path, entry } if path == new => result = entry,
                    other => out.actions.push(other),
                }
            }
            out.conflicts.extend(scratch.conflicts);
            out.actions.push(Action::Delete {
                path: old.to_string(),
            });
            out.actions.push(Action::Write {
                path: new.to_string(),
                entry: result,
            });
        }
    }
    // The project has the file, so its path is tracked.
    new_inventory.insert(new.to_string(), t.class);
}

/// Memoized project lookups for the block check.
struct Occupancy<'a> {
    project: &'a dyn ProjectView,
    kept: BTreeMap<String, bool>,
    listed: BTreeMap<String, Option<Vec<RelPath>>>,
}

impl Occupancy<'_> {
    /// True when the project has a file or symlink at `path`.
    fn has_entry(&mut self, path: &str) -> bool {
        if let Some(found) = self.kept.get(path) {
            return *found;
        }
        let found = self.project.get(path).is_some();
        self.kept.insert(path.to_string(), found);
        found
    }

    fn files_below(&mut self, dir: &str) -> Option<&Vec<RelPath>> {
        if !self.listed.contains_key(dir) {
            let files = self.project.list_files(dir);
            self.listed.insert(dir.to_string(), files);
        }
        self.listed.get(dir).and_then(Option::as_ref)
    }

    /// True when writing `path` would replace something the plan does not delete:
    /// a file at an ancestor, or files inside a directory at `path`.
    fn blocks(&mut self, path: &str, deleted: &BTreeSet<&str>) -> bool {
        let mut ancestor = String::new();
        let mut segments: Vec<&str> = path.split('/').collect();
        segments.pop();
        for segment in segments {
            if !ancestor.is_empty() {
                ancestor.push('/');
            }
            ancestor.push_str(segment);
            if !deleted.contains(ancestor.as_str()) && self.has_entry(&ancestor) {
                return true;
            }
        }
        self.files_below(path)
            .is_some_and(|files| files.iter().any(|f| !deleted.contains(f.as_str())))
    }
}

/// Drops every write whose path is occupied by something the project keeps,
/// recording a `Blocked` conflict instead. A blocked rename keeps its old name.
fn drop_blocked(
    out: &mut Out,
    inventory: &mut Inventory,
    project: &dyn ProjectView,
    pairs: &BTreeMap<&RelPath, (&RelPath, Entry)>,
) {
    let mut occupancy = Occupancy {
        project,
        kept: BTreeMap::new(),
        listed: BTreeMap::new(),
    };
    // Dropping a rename's delete can block another write, so repeat until stable.
    loop {
        let deleted: BTreeSet<&str> = out
            .actions
            .iter()
            .filter_map(|a| match a {
                Action::Delete { path } => Some(path.as_str()),
                Action::Write { .. } => None,
            })
            .collect();
        let blocked = out.actions.iter().position(
            |a| matches!(a, Action::Write { path, .. } if occupancy.blocks(path, &deleted)),
        );
        let Some(index) = blocked else { break };
        let path = out.actions.remove(index).path().to_string();
        inventory.remove(&path);
        if let Some((old, _)) = pairs.get(&path) {
            // Nothing is written for the new name: its merge outcome and side
            // file are moot, so `blocked` is the only thing reported for it.
            let side = format!("{path}{SIDE_FILE_SUFFIX}");
            out.actions.retain(|a| match a {
                Action::Delete { path: p } => p != *old,
                Action::Write { path: p, .. } => *p != side,
            });
            out.conflicts.retain(|c| c.path != path && c.path != side);
            inventory.remove(&side);
        }
        out.conflict(&path, ConflictKind::Blocked);
    }
}

/// Orders actions: every delete by path, then every write by path.
fn action_order(a: &Action, b: &Action) -> std::cmp::Ordering {
    let rank = |a: &Action| match a {
        Action::Delete { .. } => 0,
        Action::Write { .. } => 1,
    };
    rank(a).cmp(&rank(b)).then_with(|| a.path().cmp(b.path()))
}

/// Plans an update from `base` to `target` against the project's current files.
///
/// `new` is the same call with an empty base, an empty project and an empty inventory.
pub fn plan_update(
    base: &RenderedTree,
    target: &RenderedTree,
    project: &dyn ProjectView,
    inventory: &Inventory,
    target_label: &str,
) -> Plan {
    let mut out = Out::default();
    let mut new_inventory = Inventory::new();
    let paths: BTreeSet<&RelPath> = base.keys().chain(target.keys()).collect();
    let pairs = rename_pairs(base, target, project);
    let renamed: BTreeSet<&RelPath> = pairs.values().map(|(old, _)| *old).collect();

    for path in paths {
        if renamed.contains(path) {
            continue;
        }
        if let Some((old, at_old)) = pairs.get(path) {
            let (Some(b), Some(t)) = (base.get(*old), target.get(path)) else {
                continue;
            };
            let side = format!("{path}{SIDE_FILE_SUFFIX}");
            let side_taken = base.contains_key(&side) || target.contains_key(&side);
            plan_rename(
                &mut out,
                &mut new_inventory,
                old,
                path,
                b,
                t,
                at_old,
                target_label,
                side_taken,
            );
            continue;
        }
        let b = base.get(path);
        let t = target.get(path);
        let p = project.get(path);

        let Some(t) = t else {
            // Removed from the template or excluded.
            let Some(b) = b else { continue };
            if b.class == FileClass::Managed
                && let Some(p) = &p
            {
                if same_content(p, &b.entry) {
                    out.actions.push(Action::Delete { path: path.clone() });
                } else {
                    out.conflict(path, ConflictKind::RemovedByTemplate);
                }
            }
            continue;
        };

        let in_inventory = inventory.contains_key(path);
        let side = format!("{path}{SIDE_FILE_SUFFIX}");
        let side_taken = base.contains_key(&side) || target.contains_key(&side);
        let actions_before = out.actions.len();
        match t.class {
            FileClass::Starter => {
                if p.is_none() && b.is_none() && !in_inventory {
                    out.write(path, t.entry.clone(), None);
                }
            }
            FileClass::Managed => match &p {
                None => match b {
                    None => out.write(path, t.entry.clone(), None),
                    Some(b) if b.class == FileClass::Starter => {
                        out.write(path, t.entry.clone(), None)
                    }
                    Some(b) => {
                        if !same_content(&b.entry, &t.entry) {
                            out.conflict(path, ConflictKind::DeletedByProject);
                        }
                    }
                },
                Some(p) => plan_present(
                    &mut out,
                    path,
                    b.map(|b| &b.entry),
                    &t.entry,
                    p,
                    target_label,
                    side_taken,
                ),
            },
        }

        let new_actions = out.actions.get(actions_before..).unwrap_or_default();
        if in_inventory || p.is_some() || wrote_path(new_actions, path) {
            new_inventory.insert(path.clone(), t.class);
        }
    }

    drop_blocked(&mut out, &mut new_inventory, project, &pairs);
    out.actions.sort_by(action_order);
    out.conflicts.sort_by(|a, b| a.path.cmp(&b.path));
    Plan {
        actions: out.actions,
        conflicts: out.conflicts,
        inventory: new_inventory,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::render::Rendered;

    const LABEL: &str = "template v1.1.0";

    /// A key ending in `/` stands for an empty directory.
    impl ProjectView for BTreeMap<String, Entry> {
        fn get(&self, path: &str) -> Option<Entry> {
            BTreeMap::get(self, path).cloned()
        }

        fn list_files(&self, dir: &str) -> Option<Vec<RelPath>> {
            let prefix = format!("{dir}/");
            let files: Vec<RelPath> = self
                .keys()
                .filter(|k| k.starts_with(&prefix))
                .map(|k| k.trim_end_matches('/').to_string())
                .collect();
            (!files.is_empty()).then_some(files)
        }
    }

    fn file(s: &str) -> Entry {
        Entry::File {
            bytes: s.as_bytes().to_vec(),
            executable: false,
        }
    }

    fn exec(s: &str) -> Entry {
        Entry::File {
            bytes: s.as_bytes().to_vec(),
            executable: true,
        }
    }

    fn link(t: &str) -> Entry {
        Entry::Symlink { target: t.into() }
    }

    fn bin(tag: u8) -> Entry {
        Entry::File {
            bytes: vec![0, tag, 0],
            executable: false,
        }
    }

    fn tree(items: &[(&str, Entry, FileClass)]) -> RenderedTree {
        items
            .iter()
            .map(|(p, e, c)| {
                (
                    p.to_string(),
                    Rendered {
                        entry: e.clone(),
                        class: *c,
                    },
                )
            })
            .collect()
    }

    fn proj(items: &[(&str, Entry)]) -> BTreeMap<String, Entry> {
        items
            .iter()
            .map(|(p, e)| (p.to_string(), e.clone()))
            .collect()
    }

    fn inv(items: &[(&str, FileClass)]) -> Inventory {
        items.iter().map(|(p, c)| (p.to_string(), *c)).collect()
    }

    use FileClass::{Managed as M, Starter as S};

    fn run(
        base: &RenderedTree,
        target: &RenderedTree,
        p: &BTreeMap<String, Entry>,
        i: &Inventory,
    ) -> Plan {
        plan_update(base, target, p, i, LABEL)
    }

    fn write(path: &str, entry: Entry) -> Action {
        Action::Write {
            path: path.into(),
            entry,
        }
    }

    fn conflict(path: &str, kind: ConflictKind) -> Conflict {
        Conflict {
            path: path.into(),
            kind,
        }
    }

    #[test]
    fn new_managed_absent_creates() {
        let t = tree(&[("a", file("x\n"), M)]);
        let plan = run(&tree(&[]), &t, &proj(&[]), &inv(&[]));
        assert_eq!(plan.actions, vec![write("a", file("x\n"))]);
        assert!(plan.conflicts.is_empty());
        assert_eq!(plan.inventory, inv(&[("a", M)]));
    }

    #[test]
    fn new_managed_present_same_is_noop() {
        let t = tree(&[("a", file("x\n"), M)]);
        let plan = run(&tree(&[]), &t, &proj(&[("a", file("x\n"))]), &inv(&[]));
        assert!(plan.actions.is_empty());
        assert!(plan.conflicts.is_empty());
        assert_eq!(plan.inventory, inv(&[("a", M)]));
    }

    #[test]
    fn new_managed_present_differs_conflicts() {
        let t = tree(&[("a", file("x\n"), M)]);
        let plan = run(&tree(&[]), &t, &proj(&[("a", file("y\n"))]), &inv(&[]));
        assert_eq!(
            plan.conflicts,
            vec![conflict("a", ConflictKind::AddedByBoth)]
        );
    }

    #[test]
    fn added_by_both_text_conflicts() {
        let t = tree(&[("a", file("x\n"), M)]);
        let plan = run(&tree(&[]), &t, &proj(&[("a", file("y\n"))]), &inv(&[]));
        assert_eq!(
            plan.conflicts,
            vec![conflict("a", ConflictKind::AddedByBoth)]
        );
        assert_eq!(plan.actions.len(), 1);
        assert_eq!(plan.inventory, inv(&[("a", M)]));
        let Action::Write {
            entry: Entry::File { bytes, .. },
            ..
        } = &plan.actions[0]
        else {
            panic!("expected a file write");
        };
        assert!(String::from_utf8_lossy(bytes).contains("<<<<<<< project"));
    }

    #[test]
    fn new_starter_creates() {
        let t = tree(&[("a", file("x\n"), S)]);
        let plan = run(&tree(&[]), &t, &proj(&[]), &inv(&[]));
        assert_eq!(plan.actions, vec![write("a", file("x\n"))]);
        assert_eq!(plan.inventory, inv(&[("a", S)]));
    }

    #[test]
    fn starter_deleted_by_project_not_recreated() {
        let t = tree(&[("a", file("x\n"), S)]);
        let plan = run(&tree(&[]), &t, &proj(&[]), &inv(&[("a", S)]));
        assert!(plan.actions.is_empty());
        assert!(plan.conflicts.is_empty());
        assert_eq!(plan.inventory, inv(&[("a", S)]));
    }

    #[test]
    fn inventory_keeps_deleted_starter() {
        let t = tree(&[("a", file("x\n"), S)]);
        let b = tree(&[("a", file("x\n"), S)]);
        let plan = run(&b, &t, &proj(&[]), &inv(&[("a", S)]));
        assert_eq!(plan.inventory, inv(&[("a", S)]));
    }

    #[test]
    fn starter_present_untouched() {
        let t = tree(&[("a", file("new\n"), S)]);
        let b = tree(&[("a", file("old\n"), S)]);
        let plan = run(&b, &t, &proj(&[("a", file("mine\n"))]), &inv(&[("a", S)]));
        assert!(plan.actions.is_empty());
        assert!(plan.conflicts.is_empty());
    }

    #[test]
    fn managed_unchanged_by_project_gets_target() {
        let b = tree(&[("a", file("1\n"), M)]);
        let t = tree(&[("a", file("2\n"), M)]);
        let plan = run(&b, &t, &proj(&[("a", file("1\n"))]), &inv(&[("a", M)]));
        assert_eq!(plan.actions, vec![write("a", file("2\n"))]);
        assert!(plan.conflicts.is_empty());
    }

    #[test]
    fn managed_edited_merges() {
        let b = tree(&[("a", file("a\nb\nc\nd\ne\nf\ng\n"), M)]);
        let t = tree(&[("a", file("A\nb\nc\nd\ne\nf\ng\n"), M)]);
        let p = proj(&[("a", file("a\nb\nc\nd\ne\nf\nG\n"))]);
        let plan = run(&b, &t, &p, &inv(&[("a", M)]));
        assert_eq!(
            plan.actions,
            vec![write("a", file("A\nb\nc\nd\ne\nf\nG\n"))]
        );
        assert!(plan.conflicts.is_empty());
    }

    #[test]
    fn managed_edited_overlap_markers() {
        let b = tree(&[("a", file("x\n"), M)]);
        let t = tree(&[("a", file("t\n"), M)]);
        let plan = run(&b, &t, &proj(&[("a", file("p\n"))]), &inv(&[("a", M)]));
        assert_eq!(plan.conflicts, vec![conflict("a", ConflictKind::Markers)]);
        let Action::Write {
            entry: Entry::File { bytes, .. },
            ..
        } = &plan.actions[0]
        else {
            panic!("expected a file write");
        };
        let s = String::from_utf8_lossy(bytes);
        assert!(s.contains("<<<<<<< project") && s.contains(">>>>>>> template v1.1.0"));
    }

    #[test]
    fn convergent_edit_is_noop() {
        let b = tree(&[("a", file("x\n"), M), ("b", bin(1), M), ("c", link("x"), M)]);
        let t = tree(&[("a", file("t\n"), M), ("b", bin(2), M), ("c", link("y"), M)]);
        let p = proj(&[("a", file("t\n")), ("b", bin(2)), ("c", link("y"))]);
        let plan = run(&b, &t, &p, &inv(&[("a", M), ("b", M), ("c", M)]));
        assert!(plan.actions.is_empty());
        assert!(plan.conflicts.is_empty());
    }

    #[test]
    fn managed_deleted_by_project_template_same_noop() {
        let b = tree(&[("a", file("x\n"), M)]);
        let plan = run(&b, &b, &proj(&[]), &inv(&[("a", M)]));
        assert!(plan.actions.is_empty());
        assert!(plan.conflicts.is_empty());
    }

    #[test]
    fn managed_deleted_by_project_template_changed_conflicts() {
        let b = tree(&[("a", file("x\n"), M)]);
        let t = tree(&[("a", file("y\n"), M)]);
        let plan = run(&b, &t, &proj(&[]), &inv(&[("a", M)]));
        assert!(plan.actions.is_empty());
        assert_eq!(
            plan.conflicts,
            vec![conflict("a", ConflictKind::DeletedByProject)]
        );
        assert_eq!(plan.inventory, inv(&[("a", M)]));
    }

    #[test]
    fn removed_by_template_unedited_deleted() {
        let b = tree(&[("a", file("x\n"), M)]);
        let plan = run(
            &b,
            &tree(&[]),
            &proj(&[("a", file("x\n"))]),
            &inv(&[("a", M)]),
        );
        assert_eq!(plan.actions, vec![Action::Delete { path: "a".into() }]);
        assert!(plan.inventory.is_empty());
    }

    #[test]
    fn removed_by_template_edited_conflicts() {
        let b = tree(&[("a", file("x\n"), M)]);
        let plan = run(
            &b,
            &tree(&[]),
            &proj(&[("a", file("mine\n"))]),
            &inv(&[("a", M)]),
        );
        assert!(plan.actions.is_empty());
        assert_eq!(
            plan.conflicts,
            vec![conflict("a", ConflictKind::RemovedByTemplate)]
        );
        assert!(plan.inventory.is_empty());
    }

    #[test]
    fn removed_by_template_already_absent_noop() {
        let b = tree(&[("a", file("x\n"), M)]);
        let plan = run(&b, &tree(&[]), &proj(&[]), &inv(&[("a", M)]));
        assert!(plan.actions.is_empty());
        assert!(plan.conflicts.is_empty());
    }

    #[test]
    fn excluded_now_treated_as_removed() {
        // An excluded path is simply missing from the target tree.
        let b = tree(&[("a", file("x\n"), M), ("s", file("s\n"), S)]);
        let p = proj(&[("a", file("x\n")), ("s", file("s\n"))]);
        let plan = run(&b, &tree(&[]), &p, &inv(&[("a", M), ("s", S)]));
        assert_eq!(plan.actions, vec![Action::Delete { path: "a".into() }]);
        assert!(plan.conflicts.is_empty());
    }

    #[test]
    fn managed_to_starter_untouched() {
        let b = tree(&[("a", file("x\n"), M)]);
        let t = tree(&[("a", file("y\n"), S)]);
        let plan = run(&b, &t, &proj(&[("a", file("x\n"))]), &inv(&[("a", M)]));
        assert!(plan.actions.is_empty());
        assert_eq!(plan.inventory, inv(&[("a", S)]));
    }

    #[test]
    fn managed_to_starter_absent_noop() {
        let b = tree(&[("a", file("x\n"), M)]);
        let t = tree(&[("a", file("y\n"), S)]);
        let plan = run(&b, &t, &proj(&[]), &inv(&[("a", M)]));
        assert!(plan.actions.is_empty());
        assert!(plan.conflicts.is_empty());
    }

    #[test]
    fn starter_to_managed_merges() {
        let b = tree(&[("a", file("a\nb\nc\nd\ne\nf\ng\n"), S)]);
        let t = tree(&[("a", file("A\nb\nc\nd\ne\nf\ng\n"), M)]);
        let p = proj(&[("a", file("a\nb\nc\nd\ne\nf\nG\n"))]);
        let plan = run(&b, &t, &p, &inv(&[("a", S)]));
        assert_eq!(
            plan.actions,
            vec![write("a", file("A\nb\nc\nd\ne\nf\nG\n"))]
        );
        assert_eq!(plan.inventory, inv(&[("a", M)]));
    }

    #[test]
    fn starter_to_managed_absent_creates() {
        let b = tree(&[("a", file("x\n"), S)]);
        let t = tree(&[("a", file("y\n"), M)]);
        let plan = run(&b, &t, &proj(&[]), &inv(&[("a", S)]));
        assert_eq!(plan.actions, vec![write("a", file("y\n"))]);
        assert!(plan.conflicts.is_empty());
    }

    #[test]
    fn binary_diverged_writes_side_file() {
        let b = tree(&[("a.bin", bin(1), M)]);
        let t = tree(&[("a.bin", bin(2), M)]);
        let plan = run(&b, &t, &proj(&[("a.bin", bin(3))]), &inv(&[("a.bin", M)]));
        assert_eq!(plan.actions, vec![write("a.bin.rootstock-new", bin(2))]);
        assert_eq!(
            plan.conflicts,
            vec![conflict("a.bin", ConflictKind::Binary)]
        );
        assert_eq!(plan.inventory, inv(&[("a.bin", M)]));
    }

    #[test]
    fn binary_unedited_replaced() {
        let b = tree(&[("a.bin", bin(1), M)]);
        let t = tree(&[("a.bin", bin(2), M)]);
        let plan = run(&b, &t, &proj(&[("a.bin", bin(1))]), &inv(&[("a.bin", M)]));
        assert_eq!(plan.actions, vec![write("a.bin", bin(2))]);
        assert!(plan.conflicts.is_empty());
    }

    #[test]
    fn symlink_diverged_conflicts() {
        let b = tree(&[("l", link("a"), M)]);
        let t = tree(&[("l", link("b"), M)]);
        let plan = run(&b, &t, &proj(&[("l", link("c"))]), &inv(&[("l", M)]));
        assert!(plan.actions.is_empty());
        assert_eq!(plan.conflicts, vec![conflict("l", ConflictKind::Symlink)]);
    }

    #[test]
    fn kind_mismatch_conflicts_as_symlink() {
        let b = tree(&[("l", file("a\n"), M)]);
        let t = tree(&[("l", link("b"), M)]);
        let plan = run(&b, &t, &proj(&[("l", file("mine\n"))]), &inv(&[("l", M)]));
        assert!(plan.actions.is_empty());
        assert_eq!(plan.conflicts, vec![conflict("l", ConflictKind::Symlink)]);
    }

    #[test]
    fn executable_bit_follows_target() {
        let b = tree(&[("a", file("1\n"), M)]);
        let t = tree(&[("a", exec("2\n"), M)]);
        let plan = run(&b, &t, &proj(&[("a", file("1\n"))]), &inv(&[("a", M)]));
        assert_eq!(plan.actions, vec![write("a", exec("2\n"))]);
    }

    #[test]
    fn exec_bit_only_change_writes() {
        let b = tree(&[("a", file("1\n"), M)]);
        let t = tree(&[("a", exec("1\n"), M)]);
        let plan = run(&b, &t, &proj(&[("a", file("1\n"))]), &inv(&[("a", M)]));
        assert_eq!(plan.actions, vec![write("a", exec("1\n"))]);
        assert!(plan.conflicts.is_empty());
    }

    #[test]
    fn binary_project_edit_template_unchanged_is_clean() {
        let b = tree(&[("a.bin", bin(1), M)]);
        let plan = run(&b, &b, &proj(&[("a.bin", bin(9))]), &inv(&[("a.bin", M)]));
        assert!(plan.actions.is_empty());
        assert!(plan.conflicts.is_empty());
    }

    #[test]
    fn symlink_project_edit_template_unchanged_is_clean() {
        let b = tree(&[("l", link("a"), M)]);
        let plan = run(&b, &b, &proj(&[("l", link("c"))]), &inv(&[("l", M)]));
        assert!(plan.actions.is_empty());
        assert!(plan.conflicts.is_empty());
    }

    #[test]
    fn kind_mismatch_template_unchanged_is_clean() {
        let b = tree(&[("l", link("a"), M)]);
        let plan = run(&b, &b, &proj(&[("l", file("mine\n"))]), &inv(&[("l", M)]));
        assert!(plan.actions.is_empty());
        assert!(plan.conflicts.is_empty());
    }

    #[test]
    fn template_exec_bit_change_keeps_project_content() {
        let b = tree(&[("a", file("1\n"), M)]);
        let t = tree(&[("a", exec("1\n"), M)]);
        let plan = run(&b, &t, &proj(&[("a", file("mine\n"))]), &inv(&[("a", M)]));
        assert_eq!(plan.actions, vec![write("a", exec("mine\n"))]);
        assert!(plan.conflicts.is_empty());
    }

    #[test]
    fn side_file_collision_skips_side_write() {
        let b = tree(&[("a.bin", bin(1), M), ("a.bin.rootstock-new", bin(5), M)]);
        let t = tree(&[("a.bin", bin(2), M), ("a.bin.rootstock-new", bin(5), M)]);
        let p = proj(&[("a.bin", bin(3)), ("a.bin.rootstock-new", bin(5))]);
        let i = inv(&[("a.bin", M), ("a.bin.rootstock-new", M)]);
        let plan = run(&b, &t, &p, &i);
        assert!(plan.actions.is_empty());
        assert_eq!(
            plan.conflicts,
            vec![conflict("a.bin", ConflictKind::Binary)]
        );
    }

    /// A project on a case-insensitive filesystem: lookups ignore ASCII case.
    struct Folded(BTreeMap<String, Entry>);

    impl ProjectView for Folded {
        fn get(&self, path: &str) -> Option<Entry> {
            let want = path.to_lowercase();
            self.0
                .iter()
                .find(|(k, _)| k.to_lowercase() == want)
                .map(|(_, v)| v.clone())
        }

        fn list_files(&self, dir: &str) -> Option<Vec<RelPath>> {
            let prefix = format!("{}/", dir.to_lowercase());
            let files: Vec<RelPath> = self
                .0
                .keys()
                .filter(|k| k.to_lowercase().starts_with(&prefix))
                .cloned()
                .collect();
            (!files.is_empty()).then_some(files)
        }
    }

    #[test]
    fn file_becomes_dir_unedited() {
        let b = tree(&[("docs", file("d\n"), M)]);
        let t = tree(&[("docs/x", file("x\n"), M)]);
        let plan = run(
            &b,
            &t,
            &proj(&[("docs", file("d\n"))]),
            &inv(&[("docs", M)]),
        );
        assert_eq!(
            plan.actions,
            vec![delete("docs"), write("docs/x", file("x\n"))]
        );
        assert!(plan.conflicts.is_empty());
        assert_eq!(plan.inventory, inv(&[("docs/x", M)]));
    }

    #[test]
    fn file_becomes_dir_edited_blocks() {
        let b = tree(&[("docs", file("d\n"), M)]);
        let t = tree(&[("docs/x", file("x\n"), M), ("docs/y/z", file("z\n"), M)]);
        let plan = run(
            &b,
            &t,
            &proj(&[("docs", file("mine\n"))]),
            &inv(&[("docs", M)]),
        );
        assert!(plan.actions.is_empty());
        assert_eq!(
            plan.conflicts,
            vec![
                conflict("docs", ConflictKind::RemovedByTemplate),
                conflict("docs/x", ConflictKind::Blocked),
                conflict("docs/y/z", ConflictKind::Blocked),
            ]
        );
        assert!(plan.inventory.is_empty());
    }

    #[test]
    fn dir_becomes_file_unedited() {
        let b = tree(&[("docs/x", file("x\n"), M)]);
        let t = tree(&[("docs", file("d\n"), M)]);
        let plan = run(
            &b,
            &t,
            &proj(&[("docs/x", file("x\n"))]),
            &inv(&[("docs/x", M)]),
        );
        assert_eq!(
            plan.actions,
            vec![delete("docs/x"), write("docs", file("d\n"))]
        );
        assert!(plan.conflicts.is_empty());
        assert_eq!(plan.inventory, inv(&[("docs", M)]));
    }

    #[test]
    fn dir_becomes_file_with_user_file_blocks() {
        let b = tree(&[("docs/x", file("x\n"), M)]);
        let t = tree(&[("docs", file("d\n"), M)]);
        let p = proj(&[("docs/x", file("x\n")), ("docs/mine", file("m\n"))]);
        let plan = run(&b, &t, &p, &inv(&[("docs/x", M)]));
        assert_eq!(plan.actions, vec![delete("docs/x")]);
        assert_eq!(
            plan.conflicts,
            vec![conflict("docs", ConflictKind::Blocked)]
        );
        assert!(plan.inventory.is_empty());
    }

    #[test]
    fn dir_becomes_file_with_empty_subdir_blocks() {
        let b = tree(&[("docs/x", file("x\n"), M)]);
        let t = tree(&[("docs", file("d\n"), M)]);
        // `docs/empty/` is an empty directory the project added.
        let p = proj(&[("docs/x", file("x\n")), ("docs/empty/", file(""))]);
        let plan = run(&b, &t, &p, &inv(&[("docs/x", M)]));
        assert_eq!(plan.actions, vec![delete("docs/x")]);
        assert_eq!(
            plan.conflicts,
            vec![conflict("docs", ConflictKind::Blocked)]
        );
        assert!(plan.inventory.is_empty());
    }

    // The blocked-rename tests use `Folded` because a pair needs the alias.
    // On a real case-insensitive filesystem `D` (a file) and `d/` (a directory)
    // cannot coexist, so these only pin the logic of `drop_blocked`.
    #[test]
    fn blocked_rename_drops_other_conflicts() {
        // Overlapping edits would give markers on the new name, but the write is blocked.
        let b = tree(&[("d/Readme.md", file("x\n"), M)]);
        let t = tree(&[("D/README.md", file("t\n"), M)]);
        let p = Folded(proj(&[("d/Readme.md", file("p\n")), ("D", file("mine\n"))]));
        let plan = plan_update(&b, &t, &p, &inv(&[("d/Readme.md", M)]), LABEL);
        assert!(plan.actions.is_empty(), "{:?}", plan.actions);
        assert_eq!(
            plan.conflicts,
            vec![conflict("D/README.md", ConflictKind::Blocked)]
        );

        // A diverged binary would get a side file; neither it nor `binary` remains.
        let b = tree(&[("d/Img.bin", bin(1), M)]);
        let t = tree(&[("D/IMG.bin", bin(2), M)]);
        let p = Folded(proj(&[("d/Img.bin", bin(3)), ("D", file("mine\n"))]));
        let plan = plan_update(&b, &t, &p, &inv(&[("d/Img.bin", M)]), LABEL);
        assert!(plan.actions.is_empty(), "{:?}", plan.actions);
        assert_eq!(
            plan.conflicts,
            vec![conflict("D/IMG.bin", ConflictKind::Blocked)]
        );
        assert!(plan.inventory.is_empty());
    }

    #[test]
    fn blocked_rename_keeps_old_name() {
        // A file the project keeps sits where the new name's directory would be.
        let b = tree(&[("d/Readme.md", file("x\n"), M)]);
        let t = tree(&[("D/README.md", file("y\n"), M)]);
        let p = Folded(proj(&[("d/Readme.md", file("x\n")), ("D", file("mine\n"))]));
        let plan = plan_update(&b, &t, &p, &inv(&[("d/Readme.md", M)]), LABEL);
        assert!(plan.actions.is_empty());
        assert_eq!(
            plan.conflicts,
            vec![conflict("D/README.md", ConflictKind::Blocked)]
        );
        assert!(plan.inventory.is_empty());
    }

    fn delete(path: &str) -> Action {
        Action::Delete { path: path.into() }
    }

    #[test]
    fn case_rename_unedited() {
        let b = tree(&[("Readme.md", file("x\n"), M)]);
        let t = tree(&[("README.md", file("y\n"), M)]);
        let p = Folded(proj(&[("Readme.md", file("x\n"))]));
        let plan = plan_update(&b, &t, &p, &inv(&[("Readme.md", M)]), LABEL);
        assert_eq!(
            plan.actions,
            vec![delete("Readme.md"), write("README.md", file("y\n"))]
        );
        assert!(plan.conflicts.is_empty());
        assert_eq!(plan.inventory, inv(&[("README.md", M)]));

        // Same content: the name still changes.
        let t = tree(&[("README.md", file("x\n"), M)]);
        let plan = plan_update(&b, &t, &p, &inv(&[("Readme.md", M)]), LABEL);
        assert_eq!(
            plan.actions,
            vec![delete("Readme.md"), write("README.md", file("x\n"))]
        );
        assert!(plan.conflicts.is_empty());
    }

    #[test]
    fn case_rename_edited_merges() {
        let b = tree(&[("Readme.md", file("a\nb\nc\nd\ne\nf\ng\n"), M)]);
        let t = tree(&[("README.md", file("A\nb\nc\nd\ne\nf\ng\n"), M)]);
        let p = Folded(proj(&[("Readme.md", file("a\nb\nc\nd\ne\nf\nG\n"))]));
        let plan = plan_update(&b, &t, &p, &inv(&[("Readme.md", M)]), LABEL);
        assert_eq!(
            plan.actions,
            vec![
                delete("Readme.md"),
                write("README.md", file("A\nb\nc\nd\ne\nf\nG\n"))
            ]
        );
        assert!(plan.conflicts.is_empty());
        assert_eq!(plan.inventory, inv(&[("README.md", M)]));

        // The template did not change the content: the project's edit moves to the new name.
        let t = tree(&[("README.md", file("a\nb\nc\nd\ne\nf\ng\n"), M)]);
        let plan = plan_update(&b, &t, &p, &inv(&[("Readme.md", M)]), LABEL);
        assert_eq!(
            plan.actions,
            vec![
                delete("Readme.md"),
                write("README.md", file("a\nb\nc\nd\ne\nf\nG\n"))
            ]
        );
    }

    #[test]
    fn case_rename_deleted_by_project() {
        // Nothing at either name: no alias, so the rows apply independently.
        // The old name gives nothing; the new one is created (`— | managed | absent`).
        let b = tree(&[("Readme.md", file("x\n"), M)]);
        let t = tree(&[("README.md", file("y\n"), M)]);
        let p = Folded(proj(&[]));
        let plan = plan_update(&b, &t, &p, &inv(&[("Readme.md", M)]), LABEL);
        assert_eq!(plan.actions, vec![write("README.md", file("y\n"))]);
        assert!(plan.conflicts.is_empty());
        assert_eq!(plan.inventory, inv(&[("README.md", M)]));

        let t = tree(&[("README.md", file("x\n"), M)]);
        let plan = plan_update(&b, &t, &p, &inv(&[("Readme.md", M)]), LABEL);
        assert_eq!(plan.actions, vec![write("README.md", file("x\n"))]);
        assert!(plan.conflicts.is_empty());
    }

    #[test]
    fn case_rename_case_sensitive_is_delete_plus_add() {
        let b = tree(&[("Readme.md", file("x\n"), M)]);
        let t = tree(&[("README.md", file("y\n"), M)]);

        let p = proj(&[("Readme.md", file("x\n"))]);
        let plan = run(&b, &t, &p, &inv(&[("Readme.md", M)]));
        assert_eq!(
            plan.actions,
            vec![delete("Readme.md"), write("README.md", file("y\n"))]
        );
        assert!(plan.conflicts.is_empty());
        assert_eq!(plan.inventory, inv(&[("README.md", M)]));

        let p = proj(&[("Readme.md", file("edited\n"))]);
        let plan = run(&b, &t, &p, &inv(&[("Readme.md", M)]));
        assert_eq!(plan.actions, vec![write("README.md", file("y\n"))]);
        assert_eq!(
            plan.conflicts,
            vec![conflict("Readme.md", ConflictKind::RemovedByTemplate)]
        );
        assert_eq!(plan.inventory, inv(&[("README.md", M)]));
    }

    #[test]
    fn case_rename_starter_keeps_project_content() {
        let b = tree(&[("Notes.md", file("x\n"), S)]);
        let t = tree(&[("NOTES.md", file("y\n"), S)]);
        let p = Folded(proj(&[("Notes.md", file("mine\n"))]));
        let plan = plan_update(&b, &t, &p, &inv(&[("Notes.md", S)]), LABEL);
        assert_eq!(
            plan.actions,
            vec![delete("Notes.md"), write("NOTES.md", file("mine\n"))]
        );
        assert_eq!(plan.inventory, inv(&[("NOTES.md", S)]));

        // The project deleted the starter: no alias, so the new name is a new
        // starter file and is created (`— | starter | absent, not in inventory`).
        let plan = plan_update(&b, &t, &Folded(proj(&[])), &inv(&[("Notes.md", S)]), LABEL);
        assert_eq!(plan.actions, vec![write("NOTES.md", file("y\n"))]);
        assert!(plan.conflicts.is_empty());
        assert_eq!(plan.inventory, inv(&[("NOTES.md", S)]));
    }

    #[test]
    fn case_rename_distinct_project_file_not_overwritten() {
        // A case-sensitive project holds both names as distinct files.
        let b = tree(&[("Readme.md", file("x\n"), M)]);
        let t = tree(&[("README.md", file("y\n"), M)]);
        let p = proj(&[("Readme.md", file("x\n")), ("README.md", file("mine\n"))]);
        let plan = run(&b, &t, &p, &inv(&[("Readme.md", M)]));
        // The only write to README.md is the added-by-both merge, which keeps "mine".
        assert_eq!(
            plan.actions,
            vec![
                delete("Readme.md"),
                write(
                    "README.md",
                    file("<<<<<<< project\nmine\n=======\ny\n>>>>>>> template v1.1.0\n")
                ),
            ]
        );
        assert_eq!(
            plan.conflicts,
            vec![conflict("README.md", ConflictKind::AddedByBoth)]
        );
        assert_eq!(plan.inventory, inv(&[("README.md", M)]));

        // README.md equals the target: nothing to write. The edited old name is
        // kept and reported as removed by the template.
        let p = proj(&[("Readme.md", file("edited\n")), ("README.md", file("y\n"))]);
        let plan = run(&b, &t, &p, &inv(&[("Readme.md", M)]));
        assert!(plan.actions.is_empty(), "{:?}", plan.actions);
        assert_eq!(
            plan.conflicts,
            vec![conflict("Readme.md", ConflictKind::RemovedByTemplate)]
        );
    }

    #[test]
    fn case_rename_only_new_exists() {
        let b = tree(&[("Readme.md", file("x\n"), M)]);
        let t = tree(&[("README.md", file("y\n"), M)]);
        let p = proj(&[("README.md", file("mine\n"))]);
        let plan = run(&b, &t, &p, &inv(&[("Readme.md", M)]));
        assert_eq!(
            plan.conflicts,
            vec![conflict("README.md", ConflictKind::AddedByBoth)]
        );
        assert!(
            plan.actions
                .iter()
                .all(|a| !matches!(a, Action::Delete { .. })),
            "{:?}",
            plan.actions
        );
        assert_eq!(plan.inventory, inv(&[("README.md", M)]));

        let p = proj(&[("README.md", file("y\n"))]);
        let plan = run(&b, &t, &p, &inv(&[("Readme.md", M)]));
        assert!(plan.actions.is_empty());
        assert!(plan.conflicts.is_empty());
        assert_eq!(plan.inventory, inv(&[("README.md", M)]));
    }

    #[test]
    fn deletes_precede_writes() {
        let b = tree(&[("b", file("1\n"), M), ("docs/x", file("x\n"), M)]);
        let t = tree(&[("a", file("2\n"), M), ("docs", file("d\n"), M)]);
        let p = proj(&[("b", file("1\n")), ("docs/x", file("x\n"))]);
        let plan = run(&b, &t, &p, &inv(&[("b", M), ("docs/x", M)]));
        assert_eq!(
            plan.actions,
            vec![
                delete("b"),
                delete("docs/x"),
                write("a", file("2\n")),
                write("docs", file("d\n")),
            ]
        );
    }

    #[test]
    fn merge_write_takes_target_exec_bit() {
        let b = tree(&[("a", file("a\nb\nc\nd\ne\nf\ng\n"), M)]);
        let t = tree(&[("a", exec("A\nb\nc\nd\ne\nf\ng\n"), M)]);
        let p = proj(&[("a", file("a\nb\nc\nd\ne\nf\nG\n"))]);
        let plan = run(&b, &t, &p, &inv(&[("a", M)]));
        assert_eq!(
            plan.actions,
            vec![write("a", exec("A\nb\nc\nd\ne\nf\nG\n"))]
        );
        assert!(plan.conflicts.is_empty());
    }
}
