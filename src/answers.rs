//! Collecting answers: provided values, defaults, `when`, validators, prompting.

use anyhow::{Context, Result, anyhow, bail};
use serde_json::Value;

use crate::config::{Question, QuestionKind, TemplateConfig};
use crate::jinja::Jinja;
use crate::record::{Answers, toml_to_json};

/// Asks the user for one answer.
pub trait Prompter {
    fn ask(&mut self, q: &Question, default: Option<&Value>) -> Result<Value>;
}

/// Prompter for runs without a terminal: any question that reaches it is an error.
pub struct NoPrompt;

impl Prompter for NoPrompt {
    fn ask(&mut self, q: &Question, _default: Option<&Value>) -> Result<Value> {
        bail!(
            "question `{0}` has no answer and no default; pass --data {0}=...",
            q.name
        )
    }
}

/// Interactive prompter built on `inquire`.
pub struct TerminalPrompter;

impl Prompter for TerminalPrompter {
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
                let initial = default.map(|d| display_default(q.kind, d)).transpose()?;
                let mut p = inquire::Text::new(message);
                if let Some(d) = &initial {
                    p = p.with_default(d);
                }
                if let Some(h) = &q.help {
                    p = p.with_help_message(h);
                }
                let text = p.prompt()?;
                match q.kind {
                    QuestionKind::Str => Ok(Value::String(text)),
                    _ => coerce_str(q, &text),
                }
            }
            QuestionKind::Choice => {
                let labels: Vec<String> = q.choices.iter().map(|c| c.label.clone()).collect();
                let mut p = inquire::Select::new(message, labels);
                if let Some(i) = default.and_then(|d| choice_index(q, d)) {
                    p = p.with_starting_cursor(i);
                }
                if let Some(h) = &q.help {
                    p = p.with_help_message(h);
                }
                let picked = p.raw_prompt()?;
                choice_value(q, picked.index)
            }
            QuestionKind::MultiChoice => {
                let labels: Vec<String> = q.choices.iter().map(|c| c.label.clone()).collect();
                let preset: Vec<usize> = default
                    .and_then(Value::as_array)
                    .map(|items| items.iter().filter_map(|v| choice_index(q, v)).collect())
                    .unwrap_or_default();
                let mut p = inquire::MultiSelect::new(message, labels).with_default(&preset);
                if let Some(h) = &q.help {
                    p = p.with_help_message(h);
                }
                let picked = p.raw_prompt()?;
                let values = picked
                    .into_iter()
                    .map(|o| choice_value(q, o.index))
                    .collect::<Result<Vec<_>>>()?;
                Ok(Value::Array(values))
            }
        }
    }
}

/// Text shown as the pre-filled value of a text prompt.
fn display_default(kind: QuestionKind, value: &Value) -> Result<String> {
    Ok(match (kind, value) {
        (QuestionKind::Yaml, v) => serde_norway::to_string(v)?.trim().to_string(),
        (_, Value::String(s)) => s.clone(),
        (_, v) => v.to_string(),
    })
}

fn choice_values(q: &Question) -> Result<Vec<Value>> {
    q.choices
        .iter()
        .map(|c| toml_to_json(&q.name, &c.value).map_err(Into::into))
        .collect()
}

fn choice_index(q: &Question, value: &Value) -> Option<usize> {
    choice_values(q).ok()?.iter().position(|c| c == value)
}

fn choice_value(q: &Question, index: usize) -> Result<Value> {
    choice_values(q)?
        .into_iter()
        .nth(index)
        .ok_or_else(|| anyhow!("question `{}`: no choice at index {index}", q.name))
}

/// Result of `collect`.
#[derive(Debug)]
pub struct Collected {
    /// Answers to write to the record: asked questions only.
    pub recorded: Answers,
    /// Answers templates render with; also holds defaults of questions whose `when` is false.
    pub context: Answers,
}

fn bad_value(q: &Question, raw: &str, why: &str) -> anyhow::Error {
    anyhow!("question `{}`: bad value `{raw}`: {why}", q.name)
}

fn parse_bool(s: &str) -> Option<bool> {
    match s.trim().to_ascii_lowercase().as_str() {
        "true" | "yes" | "1" => Some(true),
        "false" | "no" | "0" => Some(false),
        _ => None,
    }
}

/// Finds the choice equal to `input`; a string input is first read as the choice's own type.
fn match_choice(q: &Question, input: &Value) -> Result<Option<Value>> {
    for choice in choice_values(q)? {
        let candidate = match (input, &choice) {
            (Value::String(s), Value::Bool(_)) => parse_bool(s).map(Value::Bool),
            (Value::String(s), Value::Number(_)) => s.trim().parse::<i64>().ok().map(Value::from),
            (v, _) => Some(v.clone()),
        };
        if candidate.as_ref() == Some(&choice) {
            return Ok(candidate);
        }
    }
    Ok(None)
}

/// Converts text (from `--data` or a rendered default) to the question's type.
fn coerce_str(q: &Question, raw: &str) -> Result<Value> {
    match q.kind {
        QuestionKind::Str => Ok(Value::String(raw.to_string())),
        QuestionKind::Bool => parse_bool(raw)
            .map(Value::Bool)
            .ok_or_else(|| bad_value(q, raw, "expected true or false")),
        QuestionKind::Int => raw
            .trim()
            .parse::<i64>()
            .map(Value::from)
            .map_err(|_| bad_value(q, raw, "expected an integer")),
        QuestionKind::Yaml => serde_norway::from_str::<Value>(raw)
            .map_err(|e| bad_value(q, raw, &format!("invalid YAML: {e}"))),
        QuestionKind::Choice => match_choice(q, &Value::String(raw.to_string()))?
            .ok_or_else(|| bad_value(q, raw, "not one of the choices")),
        QuestionKind::MultiChoice => {
            let trimmed = raw.trim();
            let items: Vec<Value> = if trimmed.starts_with('[') {
                match serde_norway::from_str::<Value>(trimmed) {
                    Ok(Value::Array(items)) => items,
                    _ => return Err(bad_value(q, raw, "invalid list")),
                }
            } else {
                trimmed
                    .split(',')
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(|s| Value::String(s.to_string()))
                    .collect()
            };
            multi_choice(q, &items)
        }
    }
}

fn multi_choice(q: &Question, items: &[Value]) -> Result<Value> {
    let mut out = Vec::new();
    for item in items {
        let shown = match item {
            Value::String(s) => s.clone(),
            other => other.to_string(),
        };
        match match_choice(q, item)? {
            Some(v) => out.push(v),
            None => return Err(bad_value(q, &shown, "not one of the choices")),
        }
    }
    Ok(Value::Array(out))
}

/// Checks a JSON value (record, prompt or non-string default) against the question's type.
fn coerce_json(q: &Question, value: &Value) -> Result<Value> {
    if let Value::String(s) = value
        && q.kind != QuestionKind::Yaml
    {
        return coerce_str(q, s);
    }
    let shown = value.to_string();
    match q.kind {
        QuestionKind::Yaml => Ok(value.clone()),
        QuestionKind::Bool if value.is_boolean() => Ok(value.clone()),
        QuestionKind::Int if value.is_i64() => Ok(value.clone()),
        QuestionKind::Choice => {
            match_choice(q, value)?.ok_or_else(|| bad_value(q, &shown, "not one of the choices"))
        }
        QuestionKind::MultiChoice => match value {
            Value::Array(items) => multi_choice(q, items),
            _ => Err(bad_value(q, &shown, "expected a list")),
        },
        QuestionKind::Str => Err(bad_value(q, &shown, "expected a string")),
        QuestionKind::Bool => Err(bad_value(q, &shown, "expected true or false")),
        QuestionKind::Int => Err(bad_value(q, &shown, "expected an integer")),
    }
}

/// Parses `name=value` pairs, coercing each value by its question's type.
pub fn parse_data(cfg: &TemplateConfig, pairs: &[String]) -> Result<Answers> {
    let mut out = Answers::new();
    for pair in pairs {
        let (name, raw) = pair
            .split_once('=')
            .ok_or_else(|| anyhow!("--data `{pair}`: expected name=value"))?;
        let q = cfg
            .questions
            .iter()
            .find(|q| q.name == name)
            .ok_or_else(|| anyhow!("--data `{name}`: the template has no such question"))?;
        out.insert(name.to_string(), coerce_str(q, raw)?);
    }
    Ok(out)
}

/// The question's default, rendered over `ctx` and coerced to its type.
fn render_default(q: &Question, jinja: &Jinja, ctx: &Answers) -> Result<Option<Value>> {
    match &q.default {
        None => Ok(None),
        Some(toml::Value::String(s)) => {
            let rendered = jinja
                .render_str(&format!("default of `{}`", q.name), s, ctx)
                .map_err(|e| anyhow!("question `{}`: default: {e}", q.name))?;
            coerce_str(q, &rendered).map(Some)
        }
        Some(other) => {
            let json = toml_to_json(&q.name, other)?;
            coerce_json(q, &json).map(Some)
        }
    }
}

/// Runs the validator with the current answer in scope; returns its message if the answer is invalid.
fn validate(q: &Question, jinja: &Jinja, ctx: &Answers, value: &Value) -> Result<Option<String>> {
    let Some(source) = &q.validator else {
        return Ok(None);
    };
    let mut scope = ctx.clone();
    scope.insert(q.name.clone(), value.clone());
    let message = jinja
        .render_str(&format!("validator of `{}`", q.name), source, &scope)
        .map_err(|e| anyhow!("question `{}`: validator: {e}", q.name))?;
    let message = message.trim();
    Ok((!message.is_empty()).then(|| message.to_string()))
}

fn invalid(q: &Question, value: &Value, message: &str) -> anyhow::Error {
    anyhow!("question `{}`: invalid value {value}: {message}", q.name)
}

/// Resolves every question in order.
pub fn collect(
    cfg: &TemplateConfig,
    jinja: &Jinja,
    known: &Answers,
    use_defaults: bool,
    prompter: &mut dyn Prompter,
) -> Result<Collected> {
    let mut recorded = Answers::new();
    let mut context = Answers::new();
    for q in &cfg.questions {
        if let Some(expr) = &q.when {
            let asked = jinja
                .eval_bool(expr, &context)
                .map_err(|e| anyhow!("question `{}`: when: {e}", q.name))?;
            if !asked {
                if let Some(default) = render_default(q, jinja, &context)? {
                    context.insert(q.name.clone(), default);
                }
                continue;
            }
        }
        let value = if let Some(provided) = known.get(&q.name) {
            let value = coerce_json(q, provided)?;
            if let Some(message) = validate(q, jinja, &context, &value)? {
                return Err(invalid(q, &value, &message));
            }
            value
        } else {
            let default = render_default(q, jinja, &context)?;
            match default.as_ref().filter(|_| use_defaults) {
                Some(value) => {
                    if let Some(message) = validate(q, jinja, &context, value)? {
                        return Err(invalid(q, value, &message));
                    }
                    value.clone()
                }
                None => loop {
                    let answer = prompter.ask(q, default.as_ref())?;
                    let value =
                        coerce_json(q, &answer).with_context(|| format!("question `{}`", q.name));
                    let value = match value {
                        Ok(v) => v,
                        Err(e) => {
                            eprintln!("{e:#}");
                            continue;
                        }
                    };
                    match validate(q, jinja, &context, &value)? {
                        None => break value,
                        Some(message) => eprintln!("{message}"),
                    }
                },
            }
        };
        context.insert(q.name.clone(), value.clone());
        recorded.insert(q.name.clone(), value);
    }
    Ok(Collected { recorded, context })
}
#[cfg(test)]
mod tests {
    use std::path::Path;

    use serde_json::json;

    use super::*;
    use crate::config;

    fn cfg(body: &str) -> TemplateConfig {
        config::parse(&format!("schema = 1\n{body}"), Path::new("rootstock.toml")).unwrap()
    }

    fn jinja(c: &TemplateConfig) -> Jinja {
        Jinja::new(&c.jinja, Path::new("."))
    }

    fn known(pairs: &[(&str, serde_json::Value)]) -> Answers {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.clone()))
            .collect()
    }

    /// Scripted prompter: pops answers in order and records what it was asked.
    struct Script {
        answers: Vec<serde_json::Value>,
        asked: Vec<String>,
    }

    impl Prompter for Script {
        fn ask(
            &mut self,
            q: &Question,
            _d: Option<&serde_json::Value>,
        ) -> Result<serde_json::Value> {
            self.asked.push(q.name.clone());
            if self.answers.is_empty() {
                return Err(anyhow!("script exhausted"));
            }
            Ok(self.answers.remove(0))
        }
    }

    const HARNESS: &str = r#"
[[questions]]
name = "project_name"
type = "str"
validator = "{% if not (project_name | regex_search('^[a-z][a-z0-9-]*$')) %}Name is kebab-case{% endif %}"

[[questions]]
name = "linear"
type = "bool"
default = false

[[questions]]
name = "tracker_prefix"
type = "str"
when = "linear"
default = "DEN"
validator = "{% if not (tracker_prefix | regex_search('^[A-Z]+$')) %}Prefix is uppercase{% endif %}"

[[questions]]
name = "pi_models"
type = "yaml"
default = { opus = "a", sonnet = "b", haiku = "c" }
"#;

    #[test]
    fn known_answer_wins() {
        let c = cfg(HARNESS);
        let k = known(&[
            ("project_name", json!("demo")),
            ("linear", json!(true)),
            ("tracker_prefix", json!("ABC")),
        ]);
        let out = collect(&c, &jinja(&c), &k, true, &mut NoPrompt).unwrap();
        assert_eq!(out.recorded["project_name"], json!("demo"));
        assert_eq!(out.recorded["linear"], json!(true));
        assert_eq!(out.recorded["tracker_prefix"], json!("ABC"));
        assert_eq!(out.context["tracker_prefix"], json!("ABC"));
    }

    #[test]
    fn default_is_templated() {
        let c = cfg(r#"
[[questions]]
name = "project_name"
type = "str"
[[questions]]
name = "app"
type = "str"
default = "{{ project_name }}-app"
"#);
        let k = known(&[("project_name", json!("demo"))]);
        let out = collect(&c, &jinja(&c), &k, true, &mut NoPrompt).unwrap();
        assert_eq!(out.recorded["app"], json!("demo-app"));
    }

    #[test]
    fn when_false_not_recorded_but_in_context() {
        let c = cfg(HARNESS);
        let k = known(&[("project_name", json!("demo")), ("linear", json!(false))]);
        let out = collect(&c, &jinja(&c), &k, true, &mut NoPrompt).unwrap();
        assert!(!out.recorded.contains_key("tracker_prefix"));
        assert_eq!(out.context["tracker_prefix"], json!("DEN"));
        assert_eq!(
            out.recorded["pi_models"],
            json!({"opus": "a", "sonnet": "b", "haiku": "c"})
        );
    }

    #[test]
    fn when_false_without_default_is_absent() {
        let c = cfg(r#"
[[questions]]
name = "linear"
type = "bool"
default = false
[[questions]]
name = "prefix"
type = "str"
when = "linear"
"#);
        let out = collect(&c, &jinja(&c), &Answers::new(), true, &mut NoPrompt).unwrap();
        assert!(!out.context.contains_key("prefix") && !out.recorded.contains_key("prefix"));
    }

    #[test]
    fn validator_message_fails_noninteractive() {
        let c = cfg(HARNESS);
        let k = known(&[("project_name", json!("Bad_Name"))]);
        let err = collect(&c, &jinja(&c), &k, true, &mut NoPrompt)
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("Name is kebab-case") && err.contains("project_name"),
            "{err}"
        );
    }

    #[test]
    fn prompted_value_failing_validator_is_asked_again() {
        let c = cfg(HARNESS);
        let mut p = Script {
            answers: vec![json!("Bad_Name"), json!("good")],
            asked: vec![],
        };
        let out = collect(&c, &jinja(&c), &Answers::new(), true, &mut p).unwrap();
        assert_eq!(out.recorded["project_name"], json!("good"));
        assert_eq!(p.asked[..2], ["project_name", "project_name"]);
    }

    #[test]
    fn missing_answer_without_default_errors() {
        let c = cfg(HARNESS);
        let err = collect(&c, &jinja(&c), &Answers::new(), true, &mut NoPrompt)
            .unwrap_err()
            .to_string();
        assert!(
            err.contains(
                "question `project_name` has no answer and no default; pass --data project_name=..."
            ),
            "{err}"
        );
    }

    #[test]
    fn parse_data_coerces_types() {
        let c = cfg(r#"
[[questions]]
name = "rust"
type = "bool"
[[questions]]
name = "n"
type = "int"
[[questions]]
name = "name"
type = "str"
[[questions]]
name = "pi_models"
type = "yaml"
[[questions]]
name = "lic"
type = "choice"
choices = ["mit", { label = "A", value = "apache" }]
[[questions]]
name = "feats"
type = "multichoice"
choices = ["a", "b", "c"]
"#);
        let pairs: Vec<String> = [
            "rust=false",
            "n=3",
            "name=a=b",
            "pi_models={opus: a, sonnet: b, haiku: c}",
            "lic=apache",
            "feats=a,c",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        let a = parse_data(&c, &pairs).unwrap();
        assert_eq!(a["rust"], json!(false));
        assert_eq!(a["n"], json!(3));
        assert_eq!(a["name"], json!("a=b"));
        assert_eq!(
            a["pi_models"],
            json!({"opus": "a", "sonnet": "b", "haiku": "c"})
        );
        assert_eq!(a["lic"], json!("apache"));
        assert_eq!(a["feats"], json!(["a", "c"]));
    }

    #[test]
    fn parse_data_rejects_bad_input() {
        let c = cfg(
            "[[questions]]\nname = \"n\"\ntype = \"int\"\n[[questions]]\nname = \"l\"\ntype = \"choice\"\nchoices = [\"x\"]\n",
        );
        for (pair, needle) in [
            ("typo=1", "typo"),
            ("noequals", "noequals"),
            ("n=abc", "abc"),
            ("l=y", "`y`"),
        ] {
            let err = parse_data(&c, &[pair.to_string()]).unwrap_err().to_string();
            assert!(err.contains(needle), "{pair}: {err}");
        }
    }

    #[test]
    fn drops_answers_to_removed_questions() {
        let c = cfg(r#"
[[questions]]
name = "name"
type = "str"
[[questions]]
name = "app"
type = "str"
default = "{{ name }}-app"
"#);
        let k = known(&[("name", json!("demo")), ("old_question", json!("x"))]);
        let out = collect(&c, &jinja(&c), &k, true, &mut NoPrompt).unwrap();
        assert!(!out.recorded.contains_key("old_question"));
        assert!(!out.context.contains_key("old_question"));
        assert_eq!(out.recorded["app"], json!("demo-app"));
        let rendered = jinja(&c)
            .render_str("t", "{{ name }}/{{ app }}", &out.context)
            .unwrap();
        assert_eq!(rendered, "demo/demo-app");
    }
}
