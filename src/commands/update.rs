//! `rootstock update`: three-way update of the project in the current directory.

use std::path::Path;

use anyhow::{Context, Result, bail};

use super::{
    CommonOptions, HookFailed, Report, Side, check_version, interactive, load_data, read_data_file,
    refuse_unsupported, require_clean_tree, target_label,
};
use crate::answers::NoPrompt;
use crate::apply::{DiskProject, GitMerge, apply, check_path, readable_parents};
use crate::config::Phase;
use crate::hooks::{check_trust, find_shell, run_trusted};
use crate::merge::has_markers;
use crate::plan::plan_update;
use crate::record::{self, FileClass, Record};
use crate::source::{Source, checkout, latest, parse_source, parse_subdir};

/// Arguments of `rootstock update`.
#[derive(Debug, Clone)]
pub struct UpdateOptions {
    pub to: Option<String>,
    pub common: CommonOptions,
}

fn refuse_leftover_markers(dir: &Path, rec: &Record) -> Result<()> {
    let mut found = Vec::new();
    for (path, class) in &rec.inventory {
        if *class != FileClass::Managed {
            continue;
        }
        // Inventory comes from a file the project owns: never read outside the project.
        if check_path(path).is_err() {
            bail!("record lists an unsafe path `{path}`");
        }
        if !readable_parents(dir, path)? {
            continue;
        }
        let full = dir.join(path);
        let meta = match std::fs::symlink_metadata(&full) {
            Ok(meta) => meta,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => bail!("cannot read `{path}`: {e}"),
        };
        if !meta.file_type().is_file() {
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

    let subdir = rec.subdir.as_deref().map(parse_subdir).transpose()?;
    let src = parse_source(&rec.source)?;
    let version = match &opts.to {
        Some(v) => {
            check_version(v, &src)?;
            v.clone()
        }
        None => latest(&src)?,
    };
    // HEAD of a local template is a moving ref: the same name may be a new commit.
    let moving = matches!(src, Source::Local(_)) && version == "HEAD";
    let at_target = version == rec.version && !moving;
    let has_data = read_data_file(&opts.common)?.is_some() || !opts.common.data.is_empty();
    if at_target && !has_data {
        eprintln!("already at {version}");
        return Ok(Report {
            version,
            conflicts: Vec::new(),
        });
    }

    // A version without `rootstock.toml` but with `copier.yml` is converted in memory.
    let mut err = std::io::stderr();
    let base_side = Side::load_any(
        checkout(&src, subdir.as_deref(), &rec.commit)?.named(&rec.version),
        &mut err,
    )?;
    // On the current version the target is the recorded commit, so a moved tag
    // cannot change the `commit` the record keeps.
    let target_checkout = if at_target {
        checkout(&src, subdir.as_deref(), &rec.commit)?.named(&version)
    } else {
        checkout(&src, subdir.as_deref(), &version)?
    };
    let target_side = Side::load_any(target_checkout, &mut err)?;
    let target_cfg = &target_side.cfg;

    let mut known = rec.answers.clone();
    known.extend(load_data(target_cfg, &opts.common)?);

    let base_collected = base_side.collect(&rec.answers, true, &mut NoPrompt)?;
    let mut prompter = super::prompter();
    let target_collected = target_side.collect(&known, opts.common.defaults, prompter.as_mut())?;
    if at_target && target_collected.recorded == base_collected.recorded {
        // The given answers equal the recorded ones: nothing to render or write.
        eprintln!("already at {version}; answers unchanged");
        return Ok(Report {
            version,
            conflicts: Vec::new(),
        });
    }
    let base = base_side.render(&base_collected.context)?;
    let target = target_side.render(&target_collected.context)?;

    let project = DiskProject::new(dir);
    let merge = GitMerge::new();
    let plan = plan_update(
        &base,
        &target,
        &project,
        &rec.inventory,
        &merge,
        &target_label(&version),
    );
    project.finish()?;
    merge.finish()?;
    refuse_unsupported(&plan)?;
    // Every fallible step runs before the first write (design rule 11).
    let record_text = record::to_text(&Record {
        schema: 1,
        source: rec.source.clone(),
        subdir: subdir.clone(),
        version: version.clone(),
        commit: target_side.commit().to_string(),
        answers: target_collected.recorded,
        inventory: plan.inventory.clone(),
    })?;
    // Found with the other checks: no `sh` means nothing is written.
    let shell = find_shell(&target_cfg.hooks, Phase::Update)?;
    // Asked last among the steps that can fail, so no one trusts a run that then errors.
    check_trust(
        &target_cfg.hooks,
        Phase::Update,
        opts.common.trust,
        interactive(),
    )?;

    apply(&plan, dir)?;
    let report = Report {
        version,
        conflicts: plan.conflicts,
    };
    if let Err(e) = run_trusted(&target_cfg.hooks, Phase::Update, dir, shell.as_ref()) {
        let e = e.context("files were written; hooks failed; the record was not updated");
        return Err(HookFailed::new(report, e).into());
    }
    record::write_text(dir, &record_text)
        .context("files were written; the record was not updated")?;
    eprintln!("updated to {}", report.version);
    Ok(report)
}
