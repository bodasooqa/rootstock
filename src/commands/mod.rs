//! The `new` and `update` commands and the steps they share.

pub mod new;
pub mod update;

use std::io::IsTerminal;
use std::path::Path;

use anyhow::{Context, Result, anyhow};

use crate::answers::{self, Collected, NoPrompt, Prompter, TerminalPrompter};
use crate::config::{self, TemplateConfig};
use crate::jinja::Jinja;
use crate::plan::Conflict;
use crate::record::{Answers, toml_to_json};
use crate::render::{self, RenderedTree};
use crate::source::Checkout;

/// What a finished command reports back to the CLI.
#[derive(Debug)]
pub struct Report {
    pub version: String,
    pub conflicts: Vec<Conflict>,
}

/// Flags shared by `new` and `update`.
#[derive(Debug, Clone, Default)]
pub struct CommonOptions {
    pub data: Vec<String>,
    pub data_file: Option<std::path::PathBuf>,
    pub defaults: bool,
    pub trust: bool,
}

/// True when standard input is a terminal, so questions and confirmations may be asked.
pub(crate) fn interactive() -> bool {
    std::io::stdin().is_terminal()
}

pub(crate) fn prompter() -> Box<dyn Prompter> {
    if interactive() {
        Box::new(TerminalPrompter)
    } else {
        Box::new(NoPrompt)
    }
}

/// Loads `rootstock.toml` from a checkout.
pub(crate) fn load_config(checkout: &Checkout) -> Result<TemplateConfig> {
    config::load(checkout.path()).with_context(|| {
        format!(
            "template checkout at commit {} has an unusable configuration",
            checkout.commit
        )
    })
}

/// Reads `--data-file` and `--data` into answers; flags win over the file.
pub(crate) fn load_data(cfg: &TemplateConfig, opts: &CommonOptions) -> Result<Answers> {
    let mut out = Answers::new();
    if let Some(file) = &opts.data_file {
        let text = std::fs::read_to_string(file)
            .with_context(|| format!("cannot read --data-file {}", file.display()))?;
        let table: toml::Table = toml::from_str(&text)
            .with_context(|| format!("invalid --data-file {}", file.display()))?;
        for (key, value) in &table {
            if !cfg.questions.iter().any(|q| &q.name == key) {
                return Err(anyhow!(
                    "--data-file {}: `{key}` is not a question of the template",
                    file.display()
                ));
            }
            out.insert(key.clone(), toml_to_json(key, value)?);
        }
    }
    out.extend(answers::parse_data(cfg, &opts.data)?);
    Ok(out)
}

/// Collects the answers of one side.
pub(crate) fn collect_side(
    cfg: &TemplateConfig,
    checkout: &Checkout,
    known: &Answers,
    use_defaults: bool,
    prompter: &mut dyn Prompter,
) -> Result<Collected> {
    let root = render::include_root(checkout.path(), cfg)?;
    let jinja = Jinja::new(&cfg.jinja, &root);
    answers::collect(cfg, &jinja, known, use_defaults, prompter)
}

/// Renders one side of the template with `ctx`.
pub(crate) fn render_side(
    cfg: &TemplateConfig,
    checkout: &Checkout,
    ctx: &Answers,
) -> Result<RenderedTree> {
    let root = render::include_root(checkout.path(), cfg)?;
    let jinja = Jinja::new(&cfg.jinja, &root);
    Ok(render::render_tree(checkout.path(), cfg, &jinja, ctx)?)
}

/// Label of the target side in conflict markers.
pub(crate) fn target_label(version: &str) -> String {
    format!("template {version}")
}

/// Directory entries other than nothing: true when `dir` exists and holds anything.
pub(crate) fn dir_has_entries(dir: &Path) -> Result<bool> {
    match std::fs::read_dir(dir) {
        Ok(mut it) => Ok(it.next().is_some()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(anyhow!("cannot read {}: {e}", dir.display())),
    }
}
