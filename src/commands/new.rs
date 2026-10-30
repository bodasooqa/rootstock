//! `rootstock new`: an update from an empty base into an empty project.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

use super::{
    CommonOptions, HookFailed, Report, Side, dir_has_entries, interactive, load_data, prompter,
    refuse_unsupported, target_label,
};
use crate::apply::{DiskProject, apply};
use crate::config::Phase;
use crate::hooks::{check_trust, run_trusted};
use crate::plan::plan_update;
use crate::record::{self, Inventory, RECORD_FILE, Record};
use crate::render::RenderedTree;
use crate::source::{Source, checkout, git, latest, parse_source};

/// Arguments of `rootstock new`.
#[derive(Debug, Clone)]
pub struct NewOptions {
    pub template: String,
    pub dir: PathBuf,
    pub version: Option<String>,
    pub git: bool,
    pub common: CommonOptions,
}

/// Source string stored in the record: local paths become absolute.
fn recorded_source(template: &str, src: &Source) -> Result<String> {
    match src {
        Source::Local(path) => Ok(path
            .canonicalize()
            .with_context(|| format!("cannot resolve {}", path.display()))?
            .to_string_lossy()
            .into_owned()),
        Source::Git(_) => Ok(template.to_string()),
    }
}

pub fn run(opts: &NewOptions) -> Result<Report> {
    let dir: &Path = &opts.dir;
    if dir.join(RECORD_FILE).exists() {
        bail!(
            "{} is already a rootstock project; use update",
            dir.display()
        );
    }
    if opts.git && dir_has_entries(dir)? {
        bail!("--git needs an empty directory");
    }

    let src = parse_source(&opts.template)?;
    let version = match &opts.version {
        Some(v) => v.clone(),
        None => latest(&src)?,
    };
    let side = Side::load(checkout(&src, &version)?)?;
    let cfg = &side.cfg;
    let source = recorded_source(&opts.template, &src)?;

    let known = load_data(cfg, &opts.common)?;
    let mut prompter = prompter();
    let collected = side.collect(&known, opts.common.defaults, prompter.as_mut())?;
    let target = side.render(&collected.context)?;

    let project = DiskProject::new(dir);
    let plan = plan_update(
        &RenderedTree::new(),
        &target,
        &project,
        &Inventory::new(),
        &target_label(&version),
    );
    check_trust(&cfg.hooks, Phase::New, opts.common.trust, interactive())?;
    project.finish()?;
    refuse_unsupported(&plan)?;
    // Every fallible step runs before the first write (design rule 11).
    let record_text = record::to_text(&Record {
        schema: 1,
        source,
        version: version.clone(),
        commit: side.commit().to_string(),
        answers: collected.recorded,
        inventory: plan.inventory.clone(),
    })?;

    std::fs::create_dir_all(dir).with_context(|| format!("cannot create {}", dir.display()))?;
    apply(&plan, dir)?;
    let report = Report {
        version,
        conflicts: plan.conflicts,
    };
    if let Err(e) = run_trusted(&cfg.hooks, Phase::New, dir) {
        let e = e.context("files were written; hooks failed; the record was not written");
        return Err(HookFailed::new(report, e).into());
    }
    record::write_text(dir, &record_text)
        .context("files were written; the record was not written")?;
    if opts.git {
        init_git(dir).context("project created; git init failed")?;
    }
    eprintln!(
        "created {} from {} {}",
        dir.display(),
        opts.template,
        report.version
    );
    Ok(report)
}

fn init_git(dir: &Path) -> Result<()> {
    git(&["-C", &dir.to_string_lossy(), "init", "-q"])?;
    Ok(())
}
