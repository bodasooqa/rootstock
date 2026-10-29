//! The `rootstock.toml` model: parsing and validation of a template's config.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Deserializer};

/// Name of the template configuration file at the template repository root.
pub const CONFIG_FILE: &str = "rootstock.toml";

/// Schema version this build understands.
const SUPPORTED_SCHEMA: u32 = 1;

/// Errors from loading or validating a template config.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("cannot read template config {}: {source}", path.display())]
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("invalid template config {}: {source}", path.display())]
    Parse {
        path: PathBuf,
        source: toml::de::Error,
    },
    #[error("{}: unsupported schema {found} (this rootstock supports schema {SUPPORTED_SCHEMA})", path.display())]
    UnsupportedSchema { path: PathBuf, found: u32 },
    #[error("{}: template requires rootstock {required} or newer, running {running}", path.display())]
    TooOld {
        path: PathBuf,
        required: semver::Version,
        running: semver::Version,
    },
    #[error("{}: question `{question}` is a {kind} question and needs a non-empty `choices` list", path.display())]
    MissingChoices {
        path: PathBuf,
        question: String,
        kind: &'static str,
    },
    #[error("{}: duplicate question name `{name}`", path.display())]
    DuplicateQuestion { path: PathBuf, name: String },
}

/// A parsed and validated `rootstock.toml`.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TemplateConfig {
    pub schema: u32,
    #[serde(default, deserialize_with = "de_version")]
    pub min_rootstock: Option<semver::Version>,
    /// Subdirectory holding the template files.
    #[serde(default = "dot")]
    pub root: String,
    /// Directory (relative to the repository root) that `{% include %}` and
    /// `{% import %}` resolve against; `.` is the repository root.
    #[serde(default = "dot")]
    pub include_root: String,
    /// Suffix marking files that are rendered.
    #[serde(default = "default_suffix")]
    pub suffix: String,
    /// Directories under `root` that are never emitted.
    #[serde(default)]
    pub partials: Vec<String>,
    #[serde(default)]
    pub jinja: JinjaOptions,
    #[serde(default)]
    pub files: FileRules,
    #[serde(default)]
    pub questions: Vec<Question>,
    #[serde(default)]
    pub hooks: Vec<Hook>,
}

/// Jinja environment options.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JinjaOptions {
    #[serde(default)]
    pub trim_blocks: bool,
    #[serde(default)]
    pub lstrip_blocks: bool,
    #[serde(default = "yes")]
    pub keep_trailing_newline: bool,
}

impl Default for JinjaOptions {
    fn default() -> Self {
        Self {
            trim_blocks: false,
            lstrip_blocks: false,
            keep_trailing_newline: true,
        }
    }
}

/// Gitignore-style pattern lists (templates over the answers).
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileRules {
    #[serde(default)]
    pub starter: Vec<String>,
    #[serde(default)]
    pub exclude: Vec<String>,
}

/// One question of the template, asked in declaration order.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(try_from = "RawQuestion")]
pub struct Question {
    pub name: String,
    #[serde(rename = "type")]
    pub kind: QuestionKind,
    #[serde(default)]
    pub help: Option<String>,
    /// A string default is a template over earlier answers.
    #[serde(default)]
    pub default: Option<toml::Value>,
    /// Expression (no braces); the question is asked only when it is true.
    #[serde(default)]
    pub when: Option<String>,
    /// Template rendering to an error message, empty when the answer is valid.
    #[serde(default)]
    pub validator: Option<String>,
    pub choices: Vec<Choice>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawQuestion {
    name: String,
    #[serde(rename = "type")]
    kind: QuestionKind,
    #[serde(default)]
    help: Option<String>,
    #[serde(default)]
    default: Option<toml::Value>,
    #[serde(default)]
    when: Option<String>,
    #[serde(default)]
    validator: Option<String>,
    #[serde(default)]
    choices: Vec<toml::Value>,
}

impl TryFrom<RawQuestion> for Question {
    type Error = String;

    fn try_from(raw: RawQuestion) -> Result<Self, String> {
        let choices = raw
            .choices
            .into_iter()
            .map(|c| Choice::from_toml(&raw.name, c))
            .collect::<Result<_, _>>()?;
        Ok(Question {
            name: raw.name,
            kind: raw.kind,
            help: raw.help,
            default: raw.default,
            when: raw.when,
            validator: raw.validator,
            choices,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum QuestionKind {
    Bool,
    Str,
    Int,
    Choice,
    MultiChoice,
    Yaml,
}

impl QuestionKind {
    fn name(self) -> &'static str {
        match self {
            Self::Bool => "bool",
            Self::Str => "str",
            Self::Int => "int",
            Self::Choice => "choice",
            Self::MultiChoice => "multichoice",
            Self::Yaml => "yaml",
        }
    }
}

/// A choice: a plain TOML value (label = its display string) or `{ label, value }`.
#[derive(Debug, Clone, PartialEq)]
pub struct Choice {
    pub label: String,
    pub value: toml::Value,
}

impl Choice {
    /// A plain choice is a scalar; a table must be exactly `{ label, value }`.
    fn from_toml(question: &str, raw: toml::Value) -> Result<Self, String> {
        let bad = || {
            format!(
                "question `{question}`: choice must be a plain value or a table with exactly `label` and `value`"
            )
        };
        match raw {
            toml::Value::Table(mut t) => {
                if t.len() != 2 {
                    return Err(bad());
                }
                match (t.remove("label"), t.remove("value")) {
                    (Some(toml::Value::String(label)), Some(value)) => Ok(Choice { label, value }),
                    _ => Err(bad()),
                }
            }
            toml::Value::Array(_) | toml::Value::Datetime(_) => Err(bad()),
            value => Ok(Choice {
                label: display(&value),
                value,
            }),
        }
    }
}

fn display(value: &toml::Value) -> String {
    match value {
        toml::Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// A shell command run in the project directory after files are written.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Hook {
    pub run: String,
    /// Phases the hook runs in; both when omitted.
    #[serde(default = "both_phases")]
    pub when: Vec<Phase>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Phase {
    New,
    Update,
}

fn dot() -> String {
    ".".to_string()
}

fn default_suffix() -> String {
    ".jinja".to_string()
}

fn yes() -> bool {
    true
}

fn both_phases() -> Vec<Phase> {
    vec![Phase::New, Phase::Update]
}

fn de_version<'de, D: Deserializer<'de>>(d: D) -> Result<Option<semver::Version>, D::Error> {
    let text = Option::<String>::deserialize(d)?;
    text.map(|t| semver::Version::parse(&t).map_err(serde::de::Error::custom))
        .transpose()
}

/// Reads and validates `<template_dir>/rootstock.toml`.
pub fn load(template_dir: &Path) -> Result<TemplateConfig, ConfigError> {
    let path = template_dir.join(CONFIG_FILE);
    let text = std::fs::read_to_string(&path).map_err(|source| ConfigError::Read {
        path: path.clone(),
        source,
    })?;
    parse(&text, &path)
}

/// Parses and validates config text; `path` is used in error messages only.
pub fn parse(text: &str, path: &Path) -> Result<TemplateConfig, ConfigError> {
    let config: TemplateConfig = toml::from_str(text).map_err(|source| ConfigError::Parse {
        path: path.to_path_buf(),
        source,
    })?;
    validate(&config, path)?;
    Ok(config)
}

fn validate(config: &TemplateConfig, path: &Path) -> Result<(), ConfigError> {
    if config.schema != SUPPORTED_SCHEMA {
        return Err(ConfigError::UnsupportedSchema {
            path: path.to_path_buf(),
            found: config.schema,
        });
    }
    if let Some(required) = &config.min_rootstock {
        // The crate version is a compile-time constant that parses.
        if let Ok(running) = semver::Version::parse(env!("CARGO_PKG_VERSION"))
            && required > &running
        {
            return Err(ConfigError::TooOld {
                path: path.to_path_buf(),
                required: required.clone(),
                running,
            });
        }
    }
    let mut seen = HashSet::new();
    for q in &config.questions {
        if !seen.insert(q.name.as_str()) {
            return Err(ConfigError::DuplicateQuestion {
                path: path.to_path_buf(),
                name: q.name.clone(),
            });
        }
        let needs_choices = matches!(q.kind, QuestionKind::Choice | QuestionKind::MultiChoice);
        if needs_choices && q.choices.is_empty() {
            return Err(ConfigError::MissingChoices {
                path: path.to_path_buf(),
                question: q.name.clone(),
                kind: q.kind.name(),
            });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p() -> &'static Path {
        Path::new("rootstock.toml")
    }

    const FULL: &str = r#"
schema = 1
min_rootstock = "0.1.0"
root = "template"
include_root = "partials_src"
suffix = ".tmpl"
partials = ["_partials", "_macros"]

[jinja]
trim_blocks = true
lstrip_blocks = true
keep_trailing_newline = false

[files]
starter = ["README.md", "{{ spec_path }}/**"]
exclude = ["*.bak"]

[[questions]]
name = "project_name"
type = "str"
help = "Project name"
default = "demo"
validator = "{% if not x %}bad{% endif %}"

[[questions]]
name = "rust"
type = "bool"
default = true
when = "project_name != ''"

[[questions]]
name = "license"
type = "choice"
default = "mit"
choices = ["mit", { label = "Apache 2", value = "apache" }, 3]

[[questions]]
name = "features"
type = "multichoice"
choices = [{ label = "A", value = "a" }]

[[questions]]
name = "workers"
type = "int"
default = 4

[[questions]]
name = "extra"
type = "yaml"

[[hooks]]
run = "git init"
when = ["new"]

[[hooks]]
run = "make fmt"
"#;

    #[test]
    fn parses_full_example() {
        let c = parse(FULL, p()).unwrap();
        assert_eq!(c.schema, 1);
        assert_eq!(c.min_rootstock, Some(semver::Version::new(0, 1, 0)));
        assert_eq!(c.root, "template");
        assert_eq!(c.include_root, "partials_src");
        assert_eq!(c.suffix, ".tmpl");
        assert_eq!(c.partials, vec!["_partials", "_macros"]);
        assert!(c.jinja.trim_blocks && c.jinja.lstrip_blocks && !c.jinja.keep_trailing_newline);
        assert_eq!(c.files.starter, vec!["README.md", "{{ spec_path }}/**"]);
        assert_eq!(c.files.exclude, vec!["*.bak"]);
        assert_eq!(c.questions.len(), 6);
        let names: Vec<_> = c.questions.iter().map(|q| q.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "project_name",
                "rust",
                "license",
                "features",
                "workers",
                "extra"
            ]
        );
        let q = &c.questions[0];
        assert_eq!(q.kind, QuestionKind::Str);
        assert_eq!(q.help.as_deref(), Some("Project name"));
        assert_eq!(q.default, Some(toml::Value::String("demo".into())));
        assert_eq!(q.validator.as_deref(), Some("{% if not x %}bad{% endif %}"));
        assert_eq!(q.when, None);
        let q = &c.questions[1];
        assert_eq!(q.kind, QuestionKind::Bool);
        assert_eq!(q.default, Some(toml::Value::Boolean(true)));
        assert_eq!(q.when.as_deref(), Some("project_name != ''"));
        let q = &c.questions[2];
        assert_eq!(q.kind, QuestionKind::Choice);
        assert_eq!(
            q.choices,
            vec![
                Choice {
                    label: "mit".into(),
                    value: "mit".into()
                },
                Choice {
                    label: "Apache 2".into(),
                    value: "apache".into()
                },
                Choice {
                    label: "3".into(),
                    value: toml::Value::Integer(3)
                },
            ]
        );
        assert_eq!(c.questions[3].kind, QuestionKind::MultiChoice);
        assert_eq!(c.questions[4].kind, QuestionKind::Int);
        assert_eq!(c.questions[4].default, Some(toml::Value::Integer(4)));
        assert_eq!(c.questions[5].kind, QuestionKind::Yaml);
        assert_eq!(c.hooks.len(), 2);
        assert_eq!(c.hooks[0].run, "git init");
        assert_eq!(c.hooks[0].when, vec![Phase::New]);
        assert_eq!(c.hooks[1].when, vec![Phase::New, Phase::Update]);
    }

    #[test]
    fn applies_defaults() {
        let c = parse("schema = 1", p()).unwrap();
        assert_eq!(c.root, ".");
        assert_eq!(c.include_root, ".");
        assert_eq!(c.suffix, ".jinja");
        assert!(c.jinja.keep_trailing_newline);
        assert!(!c.jinja.trim_blocks && !c.jinja.lstrip_blocks);
        assert!(c.partials.is_empty() && c.questions.is_empty() && c.hooks.is_empty());
        assert!(c.files.starter.is_empty() && c.files.exclude.is_empty());
        assert_eq!(c.min_rootstock, None);
    }

    #[test]
    fn rejects_unknown_schema() {
        let e = parse("schema = 2", p()).unwrap_err().to_string();
        assert!(e.contains("schema 2"), "{e}");
    }

    #[test]
    fn rejects_newer_min_version() {
        let e = parse("schema = 1\nmin_rootstock = \"99.0.0\"", p())
            .unwrap_err()
            .to_string();
        assert!(e.contains("99.0.0"), "{e}");
        assert!(e.contains(env!("CARGO_PKG_VERSION")), "{e}");
    }

    #[test]
    fn rejects_choice_question_without_choices() {
        for kind in ["choice", "multichoice"] {
            let text = format!("schema = 1\n[[questions]]\nname = \"a\"\ntype = \"{kind}\"");
            let e = parse(&text, p()).unwrap_err().to_string();
            assert!(e.contains("`a`") && e.contains("choices"), "{e}");
        }
    }

    #[test]
    fn rejects_duplicate_question_names() {
        let text = "schema = 1\n[[questions]]\nname = \"a\"\ntype = \"str\"\n[[questions]]\nname = \"a\"\ntype = \"bool\"";
        let e = parse(text, p()).unwrap_err().to_string();
        assert!(e.contains("duplicate") && e.contains("`a`"), "{e}");
    }

    #[test]
    fn rejects_malformed_choices() {
        for bad in [
            "{ lable = \"A\", value = \"a\" }",
            "{ label = \"A\" }",
            "{ label = \"A\", value = \"a\", extra = 1 }",
            "[\"a\", \"b\"]",
        ] {
            let text = format!(
                "schema = 1\n[[questions]]\nname = \"x\"\ntype = \"choice\"\nchoices = [{bad}]"
            );
            let e = parse(&text, p()).unwrap_err().to_string();
            assert!(
                e.contains("question `x`: choice must be a plain value"),
                "{bad}: {e}"
            );
        }
    }

    #[test]
    fn rejects_unknown_fields() {
        assert!(parse("schema = 1\nbogus = 1", p()).is_err());
    }

    #[test]
    fn load_reads_file_and_names_missing_path() {
        let dir = tempfile::tempdir().unwrap();
        let missing = load(dir.path()).unwrap_err().to_string();
        assert!(missing.contains("rootstock.toml"), "{missing}");
        assert!(
            missing.contains(&dir.path().display().to_string()),
            "{missing}"
        );
        std::fs::write(
            dir.path().join("rootstock.toml"),
            "schema = 1\nroot = \"t\"",
        )
        .unwrap();
        assert_eq!(load(dir.path()).unwrap().root, "t");
    }
}
