//! `rootstock check`: finds unresolved conflicts left in the project, for CI.

use std::path::Path;

use anyhow::Result;

use super::is_version_name;
use crate::apply::{DiskProject, check_path};
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
    if !is_version_name(&rec.version) {
        // The empty path sorts first.
        problems.push((String::new(), format!("version {}", rec.version)));
    }
    for (path, class) in &rec.inventory {
        if check_path(path).is_err() {
            // Never read an unsafe path, not even for its side file.
            problems.push((path.clone(), format!("unsafe-path {path}")));
            continue;
        }
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
