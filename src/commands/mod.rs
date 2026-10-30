//! The `new` and `update` commands and the steps they share.

pub mod check;
pub mod new;
pub mod status;
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

/// Reads and parses `--data-file`, if given.
pub(crate) fn read_data_file(opts: &CommonOptions) -> Result<Option<toml::Table>> {
    let Some(file) = &opts.data_file else {
        return Ok(None);
    };
    let text = std::fs::read_to_string(file)
        .with_context(|| format!("cannot read --data-file {}", file.display()))?;
    let table =
        toml::from_str(&text).with_context(|| format!("invalid --data-file {}", file.display()))?;
    Ok(Some(table))
}

/// Reads `--data-file` and `--data` into answers; flags win over the file.
pub(crate) fn load_data(cfg: &TemplateConfig, opts: &CommonOptions) -> Result<Answers> {
    let mut out = Answers::new();
    if let (Some(file), Some(table)) = (&opts.data_file, read_data_file(opts)?) {
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

/// One side of an update (base or target): a checkout, its config and its Jinja environment.
pub(crate) struct Side {
    pub cfg: TemplateConfig,
    checkout: Checkout,
    jinja: Jinja,
}

impl Side {
    /// Loads the config of `checkout` and builds the environment once.
    pub fn load(checkout: Checkout) -> Result<Side> {
        let cfg = load_config(&checkout)?;
        let root = render::include_root(checkout.path(), &cfg)?;
        let jinja = Jinja::new(&cfg.jinja, &root);
        Ok(Side {
            cfg,
            checkout,
            jinja,
        })
    }

    pub fn commit(&self) -> &str {
        &self.checkout.commit
    }

    /// Collects the answers of this side.
    pub fn collect(
        &self,
        known: &Answers,
        use_defaults: bool,
        prompter: &mut dyn Prompter,
    ) -> Result<Collected> {
        answers::collect(&self.cfg, &self.jinja, known, use_defaults, prompter)
    }

    /// Renders this side with `ctx`.
    pub fn render(&self, ctx: &Answers) -> Result<RenderedTree> {
        Ok(render::render_tree(
            self.checkout.path(),
            &self.cfg,
            &self.jinja,
            ctx,
        )?)
    }
}

/// Label of the target side in conflict markers.
pub(crate) fn target_label(version: &str) -> String {
    format!("template {version}")
}

/// True when `dir` exists and holds at least one entry.
pub(crate) fn dir_has_entries(dir: &Path) -> Result<bool> {
    match std::fs::read_dir(dir) {
        Ok(mut it) => Ok(it.next().is_some()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(anyhow!("cannot read {}: {e}", dir.display())),
    }
}
