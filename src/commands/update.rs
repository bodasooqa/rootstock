//! `rootstock update`: three-way update of the project in the current directory.

use std::path::Path;

use anyhow::{Context, Result, bail};

use super::{CommonOptions, Report, Side, interactive, load_data, read_data_file, target_label};
use crate::answers::NoPrompt;
use crate::apply::{DiskProject, apply, check_path, existing_parents};
use crate::config::Phase;
use crate::hooks::{check_trust, run_trusted};
use crate::merge::has_markers;
use crate::plan::plan_update;
use crate::record::{self, FileClass, Record};
use crate::source::{checkout, git, latest, parse_source, run_git};

/// Arguments of `rootstock update`.
#[derive(Debug, Clone)]
pub struct UpdateOptions {
    pub to: Option<String>,
    pub common: CommonOptions,
}

fn require_clean_tree(dir: &Path) -> Result<()> {
    let dir_str = dir.to_string_lossy();
    // A spawn failure is an error of its own; a non-zero exit carries git's reason.
    let out = run_git(&["-C", &dir_str, "rev-parse", "--is-inside-work-tree"])?;
    if !out.status.success() {
        bail!(
            "not a git work tree: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    if String::from_utf8_lossy(&out.stdout).trim() != "true" {
        bail!("not a git work tree");
    }
    let status = git(&[
        "-C",
        &dir_str,
        "status",
        "--porcelain",
        "--untracked-files=all",
        "--",
        ".",
    ])?;
    if !status.is_empty() {
        bail!("working tree has uncommitted changes; commit or stash first");
    }
    Ok(())
}

fn refuse_leftover_markers(dir: &Path, rec: &Record) -> Result<()> {
    let mut found = Vec::new();
    for (path, class) in &rec.inventory {
        if *class != FileClass::Managed {
            continue;
        }
        // Inventory comes from a file the project owns: never read outside the project.
        if check_path(path).is_err()
            || !matches!(existing_parents(dir, path, "symlink"), Ok(Some(())))
        {
            continue;
        }
        let full = dir.join(path);
        let is_file = std::fs::symlink_metadata(&full)
            .map(|m| m.file_type().is_file())
            .unwrap_or(false);
        if !is_file {
            continue;
        }
        let bytes = std::fs::read(&full).with_context(|| format!("cannot read {path}"))?;
        if has_markers(&bytes) {
            found.push(path.as_str());
        }
    }
    if !found.is_empty() {
        bail!(
            "unresolved conflict markers; resolve them and commit first:\n  {}",
            found.join("\n  ")
        );
    }
    Ok(())
}

pub fn run(dir: &Path, opts: &UpdateOptions) -> Result<Report> {
    let rec = record::read(dir)?;
    require_clean_tree(dir)?;
    refuse_leftover_markers(dir, &rec)?;

    let src = parse_source(&rec.source);
    let version = match &opts.to {
        Some(v) => v.clone(),
        None => latest(&src)?,
    };
    if version == rec.version {
        if read_data_file(&opts.common)?.is_some() || !opts.common.data.is_empty() {
            eprintln!(
                "already at {version}; --data was not applied (answers change only with a version step)"
            );
        } else {
            eprintln!("already at {version}");
        }
        return Ok(Report {
            version,
            conflicts: Vec::new(),
        });
    }

    let base_side = Side::load(checkout(&src, &rec.commit)?)?;
    let target_side = Side::load(checkout(&src, &version)?)?;
    let target_cfg = &target_side.cfg;

    let mut known = rec.answers.clone();
    known.extend(load_data(target_cfg, &opts.common)?);

    let base_collected = base_side.collect(&rec.answers, true, &mut NoPrompt)?;
    let mut prompter = super::prompter();
    let target_collected = target_side.collect(&known, opts.common.defaults, prompter.as_mut())?;
    let base = base_side.render(&base_collected.context)?;
    let target = target_side.render(&target_collected.context)?;

    let project = DiskProject::new(dir);
    let plan = plan_update(
        &base,
        &target,
        &project,
        &rec.inventory,
        &target_label(&version),
    );
    check_trust(
        &target_cfg.hooks,
        Phase::Update,
        opts.common.trust,
        interactive(),
    )?;
    project.finish()?;
    // Every fallible step runs before the first write (design rule 11).
    let record_text = record::to_text(&Record {
        schema: 1,
        source: rec.source.clone(),
        version: version.clone(),
        commit: target_side.commit().to_string(),
        answers: target_collected.recorded,
        inventory: plan.inventory.clone(),
    })?;

    apply(&plan, dir)?;
    run_trusted(&target_cfg.hooks, Phase::Update, dir)
        .context("files were written; hooks failed; the record was not updated")?;
    record::write_text(dir, &record_text)
        .context("files were written; the record was not updated")?;
    eprintln!("updated to {version}");
    Ok(Report {
        version,
        conflicts: plan.conflicts,
    })
}
