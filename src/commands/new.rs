//! `rootstock new`: an update from an empty base into an empty project.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};

use super::{
    CommonOptions, Report, collect_side, dir_has_entries, interactive, load_config, load_data,
    prompter, render_side, target_label,
};
use crate::apply::{DiskProject, apply};
use crate::config::Phase;
use crate::hooks::{check_trust, run_trusted};
use crate::plan::plan_update;
use crate::record::{self, Inventory, RECORD_FILE, Record};
use crate::render::RenderedTree;
use crate::source::{Source, checkout, latest, parse_source};

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

    let src = parse_source(&opts.template);
    let version = match &opts.version {
        Some(v) => v.clone(),
        None => latest(&src)?,
    };
    let co = checkout(&src, &version)?;
    let cfg = load_config(&co)?;
    let source = recorded_source(&opts.template, &src)?;

    let known = load_data(&cfg, &opts.common)?;
    let mut prompter = prompter();
    let collected = collect_side(&cfg, &co, &known, opts.common.defaults, prompter.as_mut())?;
    let target = render_side(&cfg, &co, &collected.context)?;

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

    std::fs::create_dir_all(dir).with_context(|| format!("cannot create {}", dir.display()))?;
    apply(&plan, dir)?;
    run_trusted(&cfg.hooks, Phase::New, dir)
        .context("files were written; hooks failed; the record was not written")?;
    record::write(
        dir,
        &Record {
            schema: 1,
            source,
            version: version.clone(),
            commit: co.commit.clone(),
            answers: collected.recorded,
            inventory: plan.inventory.clone(),
        },
    )?;
    if opts.git {
        init_git(dir)?;
    }
    eprintln!("created {} from {} {version}", dir.display(), opts.template);
    Ok(Report {
        version,
        conflicts: plan.conflicts,
    })
}

fn init_git(dir: &Path) -> Result<()> {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["init", "-q"])
        .output()
        .context("cannot run git")?;
    if !out.status.success() {
        bail!(
            "git init failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(())
}
