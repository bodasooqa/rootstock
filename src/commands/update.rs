//! `rootstock update`: three-way update of the project in the current directory.

use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result, bail};

use super::{
    CommonOptions, Report, collect_side, interactive, load_config, load_data, render_side,
    target_label,
};
use crate::answers::NoPrompt;
use crate::apply::{DiskProject, apply};
use crate::config::Phase;
use crate::hooks::{check_trust, run_trusted};
use crate::merge::has_markers;
use crate::plan::plan_update;
use crate::record::{self, FileClass, Record};
use crate::source::{checkout, latest, parse_source};

/// Arguments of `rootstock update`.
#[derive(Debug, Clone)]
pub struct UpdateOptions {
    pub to: Option<String>,
    pub common: CommonOptions,
}

/// Runs git in `dir` and returns stdout; a failure carries stderr.
fn git_in(dir: &Path, args: &[&str]) -> Result<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .context("cannot run git")?;
    if !out.status.success() {
        bail!("{}", String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

fn require_clean_tree(dir: &Path) -> Result<()> {
    let inside = git_in(dir, &["rev-parse", "--is-inside-work-tree"]).unwrap_or_default();
    if inside.trim() != "true" {
        bail!("not a git work tree; update needs git to make the result reviewable");
    }
    let status = git_in(
        dir,
        &["status", "--porcelain", "--untracked-files=all", "--", "."],
    )?;
    if !status.trim().is_empty() {
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
        eprintln!("already at {version}");
        return Ok(Report {
            version,
            conflicts: Vec::new(),
        });
    }

    let base_co = checkout(&src, &rec.commit)?;
    let target_co = checkout(&src, &version)?;
    let base_cfg = load_config(&base_co)?;
    let target_cfg = load_config(&target_co)?;

    let mut known = rec.answers.clone();
    known.extend(load_data(&target_cfg, &opts.common)?);

    let base_collected = collect_side(&base_cfg, &base_co, &rec.answers, true, &mut NoPrompt)?;
    let mut prompter = super::prompter();
    let target_collected = collect_side(
        &target_cfg,
        &target_co,
        &known,
        opts.common.defaults,
        prompter.as_mut(),
    )?;
    let base = render_side(&base_cfg, &base_co, &base_collected.context)?;
    let target = render_side(&target_cfg, &target_co, &target_collected.context)?;

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

    apply(&plan, dir)?;
    run_trusted(&target_cfg.hooks, Phase::Update, dir)
        .context("files were written; hooks failed; the record was not updated")?;
    record::write(
        dir,
        &Record {
            schema: 1,
            source: rec.source.clone(),
            version: version.clone(),
            commit: target_co.commit.clone(),
            answers: target_collected.recorded,
            inventory: plan.inventory.clone(),
        },
    )?;
    eprintln!("updated to {version}");
    Ok(Report {
        version,
        conflicts: plan.conflicts,
    })
}
