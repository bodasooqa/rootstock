//! The `new` and `update` commands and the steps they share.

pub mod check;
pub mod new;
pub mod status;
pub mod update;

use std::io::IsTerminal;
use std::path::Path;

use anyhow::{Context, Result, anyhow};
use serde_json::Value;

use crate::answers::{self, Collected, NoPrompt, Prompter};
use crate::config::{self, Question, QuestionKind, TemplateConfig};
use crate::jinja::Jinja;
use crate::plan::{Action, Conflict, Plan};
use crate::record::{Answers, toml_to_json};
use crate::render::{self, Entry, RenderedTree};
use crate::source::{Checkout, Source};

/// What a finished command reports back to the CLI.
#[derive(Debug)]
pub struct Report {
    pub version: String,
    pub conflicts: Vec<Conflict>,
}

/// A hook failed after the files were written. The conflicts in `report`
/// are on disk, so the CLI still lists them before reporting the error.
#[derive(Debug, thiserror::Error)]
#[error("{error:#}")]
pub struct HookFailed {
    pub report: Report,
    error: anyhow::Error,
}

impl HookFailed {
    pub(crate) fn new(report: Report, error: anyhow::Error) -> HookFailed {
        HookFailed { report, error }
    }
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

/// Interactive prompter built on `inquire`; the terminal side of `answers`.
pub struct TerminalPrompter;

impl Prompter for TerminalPrompter {
    fn report(&mut self, message: &str) {
        eprintln!("{message}");
    }

    fn ask(&mut self, q: &Question, default: Option<&Value>) -> Result<Value> {
        let message = q.name.as_str();
        match q.kind {
            QuestionKind::Bool => {
                let mut p = inquire::Confirm::new(message);
                if let Some(b) = default.and_then(Value::as_bool) {
                    p = p.with_default(b);
                }
                if let Some(h) = &q.help {
                    p = p.with_help_message(h);
                }
                Ok(Value::Bool(p.prompt()?))
            }
            QuestionKind::Str | QuestionKind::Int | QuestionKind::Yaml => {
                let initial = default.map(|d| display_default(q.kind, d));
                let mut p = inquire::Text::new(message);
                if let Some(d) = &initial {
                    p = p.with_initial_value(d);
                }
                if let Some(h) = &q.help {
                    p = p.with_help_message(h);
                }
                let text = p.prompt()?;
                match q.kind {
                    QuestionKind::Str => Ok(Value::String(text)),
                    _ => answers::coerce_str(q, &text),
                }
            }
            QuestionKind::Choice => {
                let labels: Vec<String> = q.choices.iter().map(|c| c.label.clone()).collect();
                let mut p = inquire::Select::new(message, labels);
                if let Some(i) = default.and_then(|d| answers::choice_index(q, d)) {
                    p = p.with_starting_cursor(i);
                }
                if let Some(h) = &q.help {
                    p = p.with_help_message(h);
                }
                let picked = p.raw_prompt()?;
                answers::choice_value(q, picked.index)
            }
            QuestionKind::MultiChoice => {
                let labels: Vec<String> = q.choices.iter().map(|c| c.label.clone()).collect();
                let preset: Vec<usize> = default
                    .and_then(Value::as_array)
                    .map(|items| {
                        items
                            .iter()
                            .filter_map(|v| answers::choice_index(q, v))
                            .collect()
                    })
                    .unwrap_or_default();
                let mut p = inquire::MultiSelect::new(message, labels).with_default(&preset);
                if let Some(h) = &q.help {
                    p = p.with_help_message(h);
                }
                let picked = p.raw_prompt()?;
                let values = picked
                    .into_iter()
                    .map(|o| answers::choice_value(q, o.index))
                    .collect::<Result<Vec<_>>>()?;
                Ok(Value::Array(values))
            }
        }
    }
}

/// Text shown as the pre-filled value of a text prompt.
fn display_default(kind: QuestionKind, value: &Value) -> String {
    match (kind, value) {
        // JSON is valid single-line YAML.
        (QuestionKind::Yaml, v) => v.to_string(),
        (_, Value::String(s)) => s.clone(),
        (_, v) => v.to_string(),
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

/// Refuses, before anything is written, a plan this platform cannot apply:
/// symlink writes where symlinks are not supported.
pub(crate) fn refuse_unsupported(plan: &Plan) -> Result<()> {
    let has_symlink = plan.actions.iter().any(|a| {
        matches!(
            a,
            Action::Write {
                entry: Entry::Symlink { .. },
                ..
            }
        )
    });
    if has_symlink && !cfg!(unix) {
        return Err(anyhow!("symlinks are not supported on this platform"));
    }
    Ok(())
}

/// Refuses a user-given version that is not `v` + semver, or `HEAD` of a
/// local template. Runs before any checkout.
pub(crate) fn check_version(version: &str, src: &Source) -> Result<()> {
    if version == "HEAD" && matches!(src, Source::Local(_)) {
        return Ok(());
    }
    let semver = version
        .strip_prefix('v')
        .is_some_and(|rest| semver::Version::parse(rest).is_ok());
    if !semver {
        return Err(anyhow!(
            "version must be a tag vX.Y.Z (or HEAD for a local template)"
        ));
    }
    Ok(())
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

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn display_default_is_single_line_text() {
        assert_eq!(display_default(QuestionKind::Str, &json!("demo")), "demo");
        assert_eq!(display_default(QuestionKind::Int, &json!(42)), "42");
        let y = display_default(QuestionKind::Yaml, &json!({"opus": "a", "sonnet": "b"}));
        assert!(!y.contains('\n'), "{y}");
        assert_eq!(
            serde_norway::from_str::<Value>(&y).unwrap(),
            json!({"opus": "a", "sonnet": "b"})
        );
    }
}
