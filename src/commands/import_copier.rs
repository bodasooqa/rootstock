//! `rootstock import-copier`: converts a Copier template in a directory into
//! `rootstock.toml` (template side), or a Copier project into a rootstock
//! project (project side). The side is chosen by what the directory holds.

use std::io::Write;
use std::path::Path;

use anyhow::{Context, Result, anyhow, bail};

use super::new::recorded_source;
use super::{
    Side, copier_file, count_findings, exists, report_findings, require_clean_tree, target_label,
};
use crate::answers::NoPrompt;
use crate::apply::DiskProject;
use crate::config::CONFIG_FILE;
use crate::copier::{self, CopierError};
use crate::merge::{Merge3, Merged};
use crate::plan::{Conflict, ConflictKind, plan_update};
use crate::record::{self, Inventory, RECORD_FILE, Record};
use crate::render::RenderedTree;
use crate::source::{Source, checkout, parse_source};

/// The file Copier keeps the answers of a project in.
const ANSWERS_FILE: &str = ".copier-answers.yml";

/// Permissions of the written `rootstock.toml`.
#[cfg(unix)]
const CONFIG_MODE: u32 = 0o644;

/// Import needs only the inventory, so the merge result is never used.
struct KeepProject;

impl Merge3 for KeepProject {
    fn merge3(&self, _: &str, _: &[u8], project: &[u8], _: &[u8], _: &str) -> Merged {
        Merged::Clean(project.to_vec())
    }
}

/// Imports the template, the project, or both from `dir`; the summary and any findings go to `err`.
///
/// Nothing is written unless the whole conversion succeeded, for both sides
/// when both are present: the project side is prepared first (clean tree,
/// network), then the template side, and only then is anything written. On the
/// template side the answers-file template is removed last, after
/// `rootstock.toml` is in place; on the project side `.copier-answers.yml` is
/// removed after the record.
pub fn run(dir: &Path, err: &mut dyn Write) -> Result<Vec<Conflict>> {
    let template = copier_file(dir)?;
    let project = match std::fs::symlink_metadata(dir.join(ANSWERS_FILE)) {
        Ok(meta) if meta.file_type().is_file() => true,
        Ok(_) => bail!("{ANSWERS_FILE} must be a regular file"),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
        Err(e) => return Err(anyhow!(e).context(format!("cannot inspect {ANSWERS_FILE}"))),
    };
    match (template, project) {
        (Some(file), false) => {
            let prepared = prepare_template(dir, file, err)?;
            write_template(dir, &prepared, err).map(|()| Vec::new())
        }
        (None, true) => {
            let prepared = prepare_project(dir, err)?;
            write_project(dir, &prepared, err)
        }
        (Some(file), true) => {
            let project = prepare_project(dir, err)?;
            let template = prepare_template(dir, file, err)?;
            write_template(dir, &template, err).context(
                "the project side was not imported (no .rootstock.toml written); restore the tree with git (checkout and clean) before running import-copier again",
            )?;
            write_project(dir, &project, err).context(
                "rootstock.toml is already written and the answers-file template removed; restore the tree with git (checkout and clean) before running import-copier again",
            )
        }
        (None, false) => {
            bail!("nothing to import here (looked for copier.yml, copier.yaml and {ANSWERS_FILE})")
        }
    }
}

/// A checked project side, ready to be written.
struct ProjectImport {
    record_text: String,
    /// Paths the project holds that rootstock cannot lead.
    blocked: Vec<Conflict>,
}

/// A checked template side, ready to be written.
struct TemplateImport {
    config_text: String,
    /// Files the conversion removes after `rootstock.toml` is in place.
    remove: Vec<String>,
}

/// Turns a Copier project into a rootstock project at its recorded version.
///
/// Hooks do not run and project files stay as they are: the project already
/// holds Copier's render of this version. Writes nothing.
fn prepare_project(dir: &Path, err: &mut dyn Write) -> Result<ProjectImport> {
    if exists(&dir.join(RECORD_FILE))? {
        bail!("{RECORD_FILE} already exists; nothing was changed");
    }
    require_clean_tree(dir)?;
    let text = std::fs::read_to_string(dir.join(ANSWERS_FILE))
        .with_context(|| format!("cannot read {ANSWERS_FILE}"))?;
    let parsed = copier::parse_answers(&text).map_err(|e| match e {
        CopierError::Answers(_) => anyhow!(e),
        other => anyhow!(other).context(format!("cannot parse {ANSWERS_FILE}")),
    })?;

    let mut src = parse_source(&parsed.src_path)?;
    if let Source::Local(path) = &src
        && path.is_relative()
    {
        // A relative path in the file is relative to the project, not to the working directory.
        src = Source::Local(dir.join(path));
    }
    let version = parsed.commit;
    let is_tag = version
        .strip_prefix('v')
        .is_some_and(|rest| semver::Version::parse(rest).is_ok());
    if !is_tag {
        bail!("`_commit` in {ANSWERS_FILE} is `{version}`: it must be a tag vX.Y.Z");
    }
    let source = recorded_source(&parsed.src_path, &src)?;
    let side = Side::load_any(checkout(&src, None, &version)?, err)?;

    for key in parsed.answers.keys() {
        if !side.cfg.questions.iter().any(|q| &q.name == key) {
            bail!("{ANSWERS_FILE}: `{key}` is not a question of the template at {version}");
        }
    }
    let collected = side
        .collect(&parsed.answers, true, &mut NoPrompt)
        .with_context(|| format!("{ANSWERS_FILE} answers at {version}"))?;
    let rendered = side
        .render(&collected.context)
        .with_context(|| format!("cannot render template version {version}"))?;
    let disk = DiskProject::new(dir);
    // Only the inventory is used: the plan is never applied.
    let plan = plan_update(
        &RenderedTree::new(),
        &rendered,
        &disk,
        &Inventory::new(),
        &KeepProject,
        &target_label(&version),
    );
    disk.finish()?;
    // Every fallible step runs before the first write (design rule 11).
    let record_text = record::to_text(&Record {
        schema: 1,
        source,
        subdir: None,
        version,
        commit: side.commit().to_string(),
        answers: collected.recorded,
        inventory: plan.inventory.clone(),
    })?;

    // Other conflicts of an empty base are edits of the project; only a path
    // that rootstock cannot lead needs a person (design rule 11).
    let blocked: Vec<Conflict> = plan
        .conflicts
        .into_iter()
        .filter(|c| c.kind == ConflictKind::Blocked)
        .collect();
    Ok(ProjectImport {
        record_text,
        blocked,
    })
}

/// Writes the record, then removes `.copier-answers.yml`.
fn write_project(
    dir: &Path,
    prepared: &ProjectImport,
    err: &mut dyn Write,
) -> Result<Vec<Conflict>> {
    record::write_text(dir, &prepared.record_text).context("cannot write the record")?;
    let _ = writeln!(err, "wrote {RECORD_FILE}");
    std::fs::remove_file(dir.join(ANSWERS_FILE))
        .with_context(|| removal_message(&prepared.blocked))?;
    let _ = writeln!(err, "removed {ANSWERS_FILE}");
    Ok(prepared.blocked.clone())
}

/// Message for a failed removal of the answers file; names the blocked paths,
/// since the error ends the run before they are listed.
fn removal_message(blocked: &[Conflict]) -> String {
    let mut message = format!("{RECORD_FILE} is already written, but cannot remove {ANSWERS_FILE}");
    if !blocked.is_empty() {
        message.push_str("; paths held by the project and left out of the record:");
        for c in blocked {
            message.push_str(&format!("\n  {} {}", c.kind.as_str(), c.path));
        }
    }
    message
}

/// Converts the template whose `copier.yml` (or `copier.yaml`) is `copier_file`.
/// Writes nothing.
fn prepare_template(dir: &Path, copier_file: &str, err: &mut dyn Write) -> Result<TemplateImport> {
    let target = dir.join(CONFIG_FILE);
    if exists(&target)? {
        bail!("{CONFIG_FILE} already exists; nothing was changed");
    }
    let yaml = std::fs::read_to_string(dir.join(copier_file))
        .with_context(|| format!("cannot read {copier_file}"))?;
    let converted = match copier::convert(&yaml, dir) {
        Ok(converted) => converted,
        Err(CopierError::Unsupported(findings)) => {
            report_findings(copier_file, &findings, err);
            bail!(
                "{} found; nothing was written",
                count_findings(findings.len())
            );
        }
        Err(e) => return Err(anyhow!(e).context(format!("cannot convert {copier_file}"))),
    };

    Ok(TemplateImport {
        config_text: converted.text,
        remove: converted.remove,
    })
}

/// Writes `rootstock.toml`, then removes the files the conversion drops.
fn write_template(dir: &Path, prepared: &TemplateImport, err: &mut dyn Write) -> Result<()> {
    let target = dir.join(CONFIG_FILE);
    let mut tmp = tempfile::NamedTempFile::new_in(dir)
        .with_context(|| format!("cannot create a temporary file in {}", dir.display()))?;
    tmp.write_all(prepared.config_text.as_bytes())
        .context("cannot write the temporary file")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        // The temporary file is created 0600; the config is an ordinary repository file.
        tmp.as_file()
            .set_permissions(std::fs::Permissions::from_mode(CONFIG_MODE))
            .context("cannot set the permissions of the temporary file")?;
    }
    tmp.persist_noclobber(&target)
        .map_err(|e| anyhow!(e.error))
        .with_context(|| format!("cannot write {CONFIG_FILE}"))?;
    let _ = writeln!(err, "wrote {CONFIG_FILE}");

    for rel in &prepared.remove {
        std::fs::remove_file(dir.join(rel)).with_context(|| {
            format!("{CONFIG_FILE} is already written, but cannot remove {rel}")
        })?;
        let _ = writeln!(err, "removed {rel}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn removal_message_lists_blocked_paths() {
        let blocked = vec![Conflict {
            path: "a/b".to_string(),
            kind: ConflictKind::Blocked,
        }];
        let m = removal_message(&blocked);
        assert!(m.contains("already written"), "{m}");
        assert!(m.contains("blocked a/b"), "{m}");
        assert!(!removal_message(&[]).contains("blocked"));
    }
}
