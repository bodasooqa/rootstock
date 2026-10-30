//! The update planner: a pure function from base render, target render and
//! the project's current state to the actions and conflicts of an update.

use std::collections::BTreeSet;

use crate::RelPath;
use crate::merge::{Merged, is_binary, merge3};
use crate::record::{FileClass, Inventory};
use crate::render::{Entry, RenderedTree};

/// Suffix of the side file written next to a conflicting binary file.
pub const SIDE_FILE_SUFFIX: &str = ".rootstock-new";

/// Read access to the project's current files; `None` means the path is absent.
pub trait ProjectView {
    fn get(&self, path: &str) -> Option<Entry>;
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

    for path in paths {
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

        if in_inventory || p.is_some() || wrote_path(&out.actions[actions_before..], path) {
            new_inventory.insert(path.clone(), t.class);
        }
    }

    out.actions.sort_by(|a, b| a.path().cmp(b.path()));
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

    impl ProjectView for BTreeMap<String, Entry> {
        fn get(&self, path: &str) -> Option<Entry> {
            BTreeMap::get(self, path).cloned()
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
