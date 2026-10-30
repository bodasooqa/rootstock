//! `rootstock check`: finds unresolved conflicts left in the project, for CI.

use std::path::Path;

use anyhow::Result;

use crate::apply::DiskProject;
use crate::merge::has_markers;
use crate::plan::{ProjectView, SIDE_FILE_SUFFIX};
use crate::record::{self, FileClass};
use crate::render::Entry;

/// Returns the report lines, sorted by path; an empty list means the project is clean.
///
/// Symlinks are never followed or scanned.
pub fn run(project: &Path) -> Result<Vec<String>> {
    let rec = record::read(project)?;
    let disk = DiskProject::new(project);
    let mut problems: Vec<(String, String)> = Vec::new();
    for (path, class) in &rec.inventory {
        if *class == FileClass::Managed
            && let Some(Entry::File { bytes, .. }) = disk.get(path)
            && has_markers(&bytes)
        {
            problems.push((path.clone(), format!("markers {path}")));
        }
        let side = format!("{path}{SIDE_FILE_SUFFIX}");
        if disk.get(&side).is_some() {
            problems.push((side.clone(), format!("side-file {side}")));
        }
    }
    disk.finish()?;
    problems.sort();
    Ok(problems.into_iter().map(|(_, line)| line).collect())
}
