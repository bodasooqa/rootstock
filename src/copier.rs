//! Conversion of a Copier template (`copier.yml`) into `rootstock.toml`.

use std::fmt;
use std::path::Path;

use crate::RelPath;
use regex::Regex;
use serde_norway::{Mapping, Value};

use crate::config::{self, ConfigError, TemplateConfig};

const HEADER: &str = "# Converted from copier.yml by rootstock import-copier.\n";

/// Suffix of templated files when `_templates_suffix` is absent (same in Copier and rootstock).
const COPIER_DEFAULT_SUFFIX: &str = ".jinja";

/// Copier 9.19 `DEFAULT_EXCLUDE`, used when neither `_subdirectory` nor `_exclude` is given.
pub const DEFAULT_EXCLUDE: [&str; 8] = [
    "copier.yaml",
    "copier.yml",
    "~*",
    "*.py[co]",
    "__pycache__",
    ".git",
    ".DS_Store",
    ".svn",
];

/// A converted template: the text of `rootstock.toml`, the same config parsed,
/// and the template files that must be removed (the Copier answers-file template).
#[derive(Debug)]
pub struct Converted {
    pub text: String,
    pub config: TemplateConfig,
    /// Paths relative to the template directory.
    pub remove: Vec<RelPath>,
}

/// One thing in the template that rootstock cannot convert.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding(pub String);

impl fmt::Display for Finding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Why a conversion did not produce a result.
#[derive(Debug, thiserror::Error)]
pub enum CopierError {
    #[error("not a single valid YAML document: {0}")]
    Yaml(#[from] serde_norway::Error),
    #[error("the top level must be a mapping")]
    NotMapping,
    #[error("cannot read {path}: {source}")]
    Io {
        path: String,
        source: std::io::Error,
    },
    #[error("the template uses features rootstock cannot convert")]
    Unsupported(Vec<Finding>),
    #[error("internal converter error: {0}")]
    Internal(String),
}

impl From<ConfigError> for CopierError {
    fn from(e: ConfigError) -> Self {
        CopierError::Internal(format!("the generated rootstock.toml is invalid: {e}"))
    }
}

/// Converts the text of `copier.yml`; `template_dir` is the directory holding it.
///
/// Reads the template tree under the template root but writes nothing.
pub fn convert(yaml: &str, template_dir: &Path) -> Result<Converted, CopierError> {
    let value: Value = serde_norway::from_str(yaml)?;
    let mut cx = Ctx::new()?;
    cx.tags(&value, "copier.yml");
    let map = match value {
        Value::Mapping(m) => m,
        Value::Null => Mapping::new(),
        _ => return Err(CopierError::NotMapping),
    };

    let mut sections: Sections = Sections::default();
    let mut questions: Vec<(&str, &Value)> = Vec::new();
    for (k, v) in &map {
        let Some(key) = k.as_str() else {
            cx.find("copier.yml: top-level key is not a string");
            continue;
        };
        match key {
            "_min_copier_version" | "_answers_file" => {}
            "_subdirectory" => sections.subdirectory = Some(v),
            "_templates_suffix" => sections.suffix = Some(v),
            "_envops" => sections.envops = Some(v),
            "_exclude" => sections.exclude = Some(v),
            "_skip_if_exists" => sections.skip = Some(v),
            "_tasks" => sections.tasks = Some(v),
            _ if key.starts_with('_') => cx.find(format!("`{key}`: key not supported")),
            _ => questions.push((key, v)),
        }
    }

    let mut table = toml::Table::new();
    table.insert("schema".into(), toml::Value::Integer(1));

    let before = cx.findings.len();
    let raw_root = cx.template_string("_subdirectory", sections.subdirectory, false);
    let mut root_valid = cx.findings.len() == before;
    let root = match raw_root {
        None => ".".to_string(),
        Some(raw) => match normalize_root(&raw) {
            Some(root) => root,
            None => {
                cx.find("`_subdirectory`: must be a relative path inside the template");
                root_valid = false;
                ".".to_string()
            }
        },
    };
    let top_level = root == ".";
    if !top_level {
        table.insert("root".into(), toml::Value::String(root.clone()));
    }
    let suffix = cx
        .template_string("_templates_suffix", sections.suffix, true)
        .unwrap_or_else(|| COPIER_DEFAULT_SUFFIX.to_string());
    table.insert("suffix".into(), toml::Value::String(suffix.clone()));
    table.insert("jinja".into(), cx.envops(sections.envops));

    let skip = cx.string_list("_skip_if_exists", sections.skip);
    for (i, entry) in skip.iter().enumerate() {
        cx.check_vars(&format!("`_skip_if_exists[{i}]`"), entry);
    }
    let mut exclude = Vec::new();
    match sections.exclude {
        Some(v) => {
            for (i, entry) in cx.string_list("_exclude", Some(v)).into_iter().enumerate() {
                match cx.update_only(&entry) {
                    Some(pair) if skip.iter().any(|s| s.trim() == pair) => {}
                    Some(_) => cx.find(format!(
                        "`_exclude[{i}]`: update-only exclude without a matching `_skip_if_exists` entry: {entry}"
                    )),
                    None => {
                        cx.check_vars(&format!("`_exclude[{i}]`"), &entry);
                        exclude.push(entry);
                    }
                }
            }
        }
        None if top_level => exclude.extend(DEFAULT_EXCLUDE.iter().map(|s| s.to_string())),
        None => {}
    }
    let mut files = toml::Table::new();
    files.insert("starter".into(), strings(&skip));
    files.insert("exclude".into(), strings(&exclude));
    table.insert("files".into(), toml::Value::Table(files));

    let hooks = cx.tasks(sections.tasks);
    if !hooks.is_empty() {
        table.insert("hooks".into(), toml::Value::Array(hooks));
    }
    let mut converted_questions = Vec::new();
    for (name, def) in questions {
        if let Some(q) = cx.question(name, def) {
            converted_questions.push(toml::Value::Table(q));
        }
    }
    if !converted_questions.is_empty() {
        table.insert("questions".into(), toml::Value::Array(converted_questions));
    }

    let mut remove = Vec::new();
    if root_valid && cx.check_root(template_dir, &root)? {
        cx.scan_tree(template_dir, &root, &suffix, &mut remove)?;
    }

    if !cx.findings.is_empty() {
        return Err(CopierError::Unsupported(cx.findings));
    }
    let body = toml::to_string(&table).map_err(|e| CopierError::Internal(e.to_string()))?;
    let text = format!("{HEADER}{body}");
    let config = config::parse(&text, Path::new(config::CONFIG_FILE))?;
    Ok(Converted {
        text,
        config,
        remove,
    })
}

#[derive(Default)]
struct Sections<'a> {
    subdirectory: Option<&'a Value>,
    suffix: Option<&'a Value>,
    envops: Option<&'a Value>,
    exclude: Option<&'a Value>,
    skip: Option<&'a Value>,
    tasks: Option<&'a Value>,
}

fn strings(items: &[String]) -> toml::Value {
    toml::Value::Array(
        items
            .iter()
            .map(|s| toml::Value::String(s.clone()))
            .collect(),
    )
}

/// Normalizes `_subdirectory` the way Copier compares it with `Path()`: no `.`
/// components, no trailing `/`, empty is `.`. `None` when it is not a relative
/// path inside the template (same rules as `root` in `config`, plus `:`).
fn normalize_root(raw: &str) -> Option<String> {
    if raw.contains(['\\', '\0', ':']) || raw.starts_with('/') {
        return None;
    }
    let mut parts = Vec::new();
    for part in raw.split('/') {
        match part {
            "" | "." => {}
            ".." => return None,
            other => parts.push(other),
        }
    }
    Some(if parts.is_empty() {
        ".".to_string()
    } else {
        parts.join("/")
    })
}

fn has_jinja(text: &str) -> bool {
    text.contains("{{") || text.contains("{%") || text.contains("{#")
}

fn regex(pattern: &str) -> Result<Regex, CopierError> {
    Regex::new(pattern).map_err(|e| CopierError::Internal(e.to_string()))
}

struct Patterns {
    var: Regex,
    update_only: Regex,
    update_only_and: Regex,
    task_when: Regex,
    loose_condition: Regex,
    when_expr: Regex,
}

impl Patterns {
    fn new() -> Result<Self, CopierError> {
        let open = r#"^\s*\{%-?\s*if\s+_copier_operation\s*==\s*(?:'update'|"update")"#;
        let close = r"\{%-?\s*endif\s*-?%\}\s*$";
        Ok(Patterns {
            var: regex(r"\b(?:_copier_\w*|_folder_name|_external_data)\b")?,
            update_only: regex(&format!(r"(?s){open}\s*-?%\}}(.*){close}"))?,
            update_only_and: regex(&format!(r"(?s){open}\s+and\s+(.+?)\s*-?%\}}(.*){close}"))?,
            loose_condition: regex(r"\b(?:or|if|else)\b")?,
            task_when: regex(
                r#"^\{\{-?\s*_copier_operation\s*==\s*(?:'(copy|update)'|"(copy|update)")\s*-?\}\}$"#,
            )?,
            when_expr: regex(r"(?s)^\{\{-?\s*(.*?)\s*-?\}\}$")?,
        })
    }
}

struct Ctx {
    pat: Patterns,
    findings: Vec<Finding>,
}

impl Ctx {
    fn new() -> Result<Self, CopierError> {
        Ok(Ctx {
            pat: Patterns::new()?,
            findings: Vec::new(),
        })
    }

    fn find(&mut self, message: impl Into<String>) {
        self.findings.push(Finding(message.into()));
    }

    /// Reports the first Copier-only variable in `text`.
    fn check_vars(&mut self, place: &str, text: &str) {
        if let Some(m) = self.pat.var.find(text) {
            let name = m.as_str();
            self.find(format!("{place}: uses Copier variable `{name}`"));
        }
    }

    /// Reports every YAML tag in the document.
    fn tags(&mut self, value: &Value, path: &str) {
        match value {
            Value::Tagged(t) => {
                self.find(format!("`{path}`: YAML tag `{}` not supported", t.tag));
            }
            Value::Sequence(items) => {
                for (i, item) in items.iter().enumerate() {
                    self.tags(item, &format!("{path}[{i}]"));
                }
            }
            Value::Mapping(map) => {
                for (k, v) in map {
                    let key = k.as_str().unwrap_or("?");
                    self.tags(v, &format!("{path}.{key}"));
                }
            }
            _ => {}
        }
    }

    /// A plain string key (`_subdirectory`, `_templates_suffix`) without Jinja.
    fn template_string(&mut self, key: &str, v: Option<&Value>, non_empty: bool) -> Option<String> {
        let v = v?;
        let Some(s) = v.as_str() else {
            self.find(format!("`{key}`: must be a string"));
            return None;
        };
        if has_jinja(s) {
            self.find(format!("`{key}`: Jinja in the value is not supported"));
            return None;
        }
        if s.is_empty() {
            if non_empty {
                self.find(format!("`{key}`: must not be empty"));
            }
            return None;
        }
        Some(s.to_string())
    }

    fn envops(&mut self, v: Option<&Value>) -> toml::Value {
        let mut trim_blocks = false;
        let mut lstrip_blocks = false;
        let mut keep_trailing_newline = true;
        if let Some(v) = v {
            match v.as_mapping() {
                None => self.find("`_envops`: must be a mapping"),
                Some(map) => {
                    for (k, val) in map {
                        let key = k.as_str().unwrap_or("?");
                        let slot = match key {
                            "trim_blocks" => &mut trim_blocks,
                            "lstrip_blocks" => &mut lstrip_blocks,
                            "keep_trailing_newline" => &mut keep_trailing_newline,
                            _ => {
                                self.findings.push(Finding(format!(
                                    "`_envops.{key}`: option not supported"
                                )));
                                continue;
                            }
                        };
                        match val.as_bool() {
                            Some(b) => *slot = b,
                            None => self
                                .findings
                                .push(Finding(format!("`_envops.{key}`: must be true or false"))),
                        }
                    }
                }
            }
        }
        let mut t = toml::Table::new();
        t.insert("trim_blocks".into(), toml::Value::Boolean(trim_blocks));
        t.insert("lstrip_blocks".into(), toml::Value::Boolean(lstrip_blocks));
        t.insert(
            "keep_trailing_newline".into(),
            toml::Value::Boolean(keep_trailing_newline),
        );
        toml::Value::Table(t)
    }

    fn string_list(&mut self, key: &str, v: Option<&Value>) -> Vec<String> {
        let Some(v) = v else {
            return Vec::new();
        };
        let Some(items) = v.as_sequence() else {
            self.find(format!("`{key}`: must be a list"));
            return Vec::new();
        };
        let mut out = Vec::new();
        for (i, item) in items.iter().enumerate() {
            match item.as_str() {
                Some(s) => out.push(s.to_string()),
                None => self.find(format!("`{key}[{i}]`: must be a string")),
            }
        }
        out
    }

    /// For an update-only exclude entry, the `_skip_if_exists` entry that makes it redundant.
    fn update_only(&self, entry: &str) -> Option<String> {
        let has_block = |p: &str| p.contains("{%") || p.contains("%}");
        if let Some(c) = self.pat.update_only.captures(entry) {
            return (!has_block(&c[1])).then(|| c[1].trim().to_string());
        }
        let c = self.pat.update_only_and.captures(entry)?;
        if has_block(&c[2]) || self.pat.loose_condition.is_match(&c[1]) {
            return None;
        }
        Some(format!("{{% if {} %}}{}{{% endif %}}", c[1].trim(), &c[2]))
    }

    fn tasks(&mut self, v: Option<&Value>) -> Vec<toml::Value> {
        let Some(v) = v else {
            return Vec::new();
        };
        let Some(items) = v.as_sequence() else {
            self.find("`_tasks`: must be a list");
            return Vec::new();
        };
        let mut hooks = Vec::new();
        for (i, item) in items.iter().enumerate() {
            let place = format!("`_tasks[{i}]`");
            if let Some(h) = self.task(&place, item) {
                hooks.push(toml::Value::Table(h));
            }
        }
        hooks
    }

    fn task(&mut self, place: &str, item: &Value) -> Option<toml::Table> {
        let before = self.findings.len();
        let mut command = None;
        let mut phases: Option<&str> = None;
        match item {
            Value::String(s) => command = Some(s.clone()),
            Value::Mapping(map) => {
                for (k, v) in map {
                    match k.as_str() {
                        Some("command") => match v {
                            Value::String(s) => command = Some(s.clone()),
                            _ => self.find(format!("{place}: a command list is not supported")),
                        },
                        Some("when") => match v {
                            Value::Bool(true) => {}
                            Value::String(s) => {
                                let op = self.pat.task_when.captures(s.trim()).map(|c| {
                                    if c.get(1).or_else(|| c.get(2)).map(|m| m.as_str())
                                        == Some("copy")
                                    {
                                        "new"
                                    } else {
                                        "update"
                                    }
                                });
                                match op {
                                    Some(p) => phases = Some(p),
                                    None => self.find(format!("{place}: `when` not supported")),
                                }
                            }
                            _ => self.find(format!("{place}: `when` not supported")),
                        },
                        other => self.find(format!(
                            "{place}: key `{}` not supported",
                            other.unwrap_or("?")
                        )),
                    }
                }
                if command.is_none() && self.findings.len() == before {
                    self.find(format!("{place}: no `command`"));
                }
            }
            _ => self.find(format!("{place}: must be a string or a mapping")),
        }
        if let Some(cmd) = &command {
            if has_jinja(cmd) {
                self.find(format!(
                    "{place}: Jinja in a command is not supported (rootstock hooks are not rendered)"
                ));
            } else {
                self.check_vars(place, cmd);
            }
        }
        if self.findings.len() != before {
            return None;
        }
        let mut hook = toml::Table::new();
        hook.insert("run".into(), toml::Value::String(command?));
        if let Some(p) = phases {
            hook.insert(
                "when".into(),
                toml::Value::Array(vec![toml::Value::String(p.to_string())]),
            );
        }
        Some(hook)
    }

    fn question(&mut self, name: &str, def: &Value) -> Option<toml::Table> {
        let before = self.findings.len();
        let place = format!("question `{name}`");
        let Some(map) = def.as_mapping() else {
            self.find(format!(
                "{place}: the short form `name: value` has no type, use a mapping"
            ));
            return None;
        };
        let mut kind: Option<String> = None;
        let mut out = toml::Table::new();
        out.insert("name".into(), toml::Value::String(name.to_string()));
        let mut choices: Option<&Value> = None;
        for (k, v) in map {
            match k.as_str() {
                Some("type") => match v.as_str() {
                    Some(t @ ("bool" | "str" | "yaml")) => kind = Some(t.to_string()),
                    Some(t) => self.find(format!("{place}: type `{t}` not supported")),
                    None => self.find(format!("{place}: `type` must be a string")),
                },
                Some(field @ ("help" | "validator")) => match v.as_str() {
                    Some(s) => {
                        self.check_vars(&place, s);
                        out.insert(field.into(), toml::Value::String(s.to_string()));
                    }
                    None => self.find(format!("{place}: `{field}` must be a string")),
                },
                Some("default") => {
                    if let Some(d) = self.yaml_value(&place, v) {
                        out.insert("default".into(), d);
                    }
                }
                Some("when") => {
                    if let Some(w) = self.question_when(&place, v) {
                        out.insert("when".into(), toml::Value::String(w));
                    }
                }
                Some("choices") => choices = Some(v),
                other => self.find(format!(
                    "{place}: field `{}` not supported",
                    other.unwrap_or("?")
                )),
            }
        }
        if kind.is_none() && !map.contains_key("type") {
            self.find(format!("{place}: no `type`"));
        }
        let mut out_kind = kind.clone();
        if let Some(c) = choices {
            if kind.as_deref() == Some("str") {
                out_kind = Some("choice".to_string());
                let list = self.choices(&place, c);
                if !list.is_empty() {
                    out.insert("choices".into(), toml::Value::Array(list));
                }
            } else {
                self.find(format!("{place}: `choices` is supported only on `str`"));
            }
        }
        if let Some(t) = out_kind {
            out.insert("type".into(), toml::Value::String(t));
        }
        (self.findings.len() == before).then_some(out)
    }

    fn question_when(&mut self, place: &str, v: &Value) -> Option<String> {
        match v {
            Value::Bool(b) => Some(b.to_string()),
            Value::String(s) => {
                self.check_vars(place, s);
                let expr = self
                    .pat
                    .when_expr
                    .captures(s.trim())
                    .map(|c| c[1].to_string())
                    .filter(|e| !e.is_empty() && !has_jinja(e) && !e.contains("}}"));
                if expr.is_none() {
                    self.find(format!("{place}: `when` not supported"));
                }
                expr
            }
            _ => {
                self.find(format!("{place}: `when` not supported"));
                None
            }
        }
    }

    fn choices(&mut self, place: &str, v: &Value) -> Vec<toml::Value> {
        let mut out = Vec::new();
        match v {
            Value::Mapping(map) => {
                for (k, val) in map {
                    let (Some(label), Some(value)) = (k.as_str(), self.scalar(place, val)) else {
                        self.find(format!(
                            "{place}: `choices` entry must be a string label with a scalar value"
                        ));
                        continue;
                    };
                    self.check_vars(place, label);
                    let mut t = toml::Table::new();
                    t.insert("label".into(), toml::Value::String(label.to_string()));
                    t.insert("value".into(), value);
                    out.push(toml::Value::Table(t));
                }
            }
            Value::Sequence(items) => {
                for item in items {
                    match self.scalar(place, item) {
                        Some(value) => out.push(value),
                        None => self.find(format!("{place}: `choices` entry must be a scalar")),
                    }
                }
            }
            _ => self.find(format!("{place}: `choices` must be a list or a mapping")),
        }
        if out.is_empty() {
            self.find(format!("{place}: `choices` must not be empty"));
        }
        out
    }

    /// A scalar YAML value as TOML; `None` for anything else, without a finding.
    fn scalar(&mut self, place: &str, v: &Value) -> Option<toml::Value> {
        match v {
            Value::Bool(_) | Value::Number(_) | Value::String(_) => self.yaml_value(place, v),
            _ => None,
        }
    }

    fn yaml_value(&mut self, place: &str, v: &Value) -> Option<toml::Value> {
        match v {
            Value::Null => {
                self.find(format!("{place}: `null` cannot be written to TOML"));
                None
            }
            Value::Bool(b) => Some(toml::Value::Boolean(*b)),
            Value::Number(n) => {
                if let Some(i) = n.as_i64() {
                    Some(toml::Value::Integer(i))
                } else if n.is_u64() {
                    self.find(format!("{place}: integer out of range"));
                    None
                } else {
                    n.as_f64().map(toml::Value::Float)
                }
            }
            Value::String(s) => {
                self.check_vars(place, s);
                Some(toml::Value::String(s.clone()))
            }
            Value::Sequence(items) => {
                let mut out = Vec::new();
                let mut ok = true;
                for item in items {
                    match self.yaml_value(place, item) {
                        Some(x) => out.push(x),
                        None => ok = false,
                    }
                }
                ok.then_some(toml::Value::Array(out))
            }
            Value::Mapping(map) => {
                let mut out = toml::Table::new();
                let mut ok = true;
                for (k, val) in map {
                    let Some(key) = k.as_str() else {
                        self.find(format!("{place}: mapping key is not a string"));
                        ok = false;
                        continue;
                    };
                    match self.yaml_value(place, val) {
                        Some(x) => {
                            out.insert(key.to_string(), x);
                        }
                        None => ok = false,
                    }
                }
                ok.then_some(toml::Value::Table(out))
            }
            // Already reported by the tag walk.
            Value::Tagged(_) => None,
        }
    }

    /// Checks that the template root is a real directory inside the template:
    /// every prefix is looked at without following links. Returns false (with a
    /// finding) when the tree must not be walked.
    fn check_root(&mut self, template_dir: &Path, root: &str) -> Result<bool, CopierError> {
        if root == "." {
            return Ok(true);
        }
        let mut prefix = String::new();
        for part in root.split('/') {
            if !prefix.is_empty() {
                prefix.push('/');
            }
            prefix.push_str(part);
            let path = template_dir.join(&prefix);
            let meta = match std::fs::symlink_metadata(&path) {
                Ok(meta) => meta,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    self.find(format!(
                        "`_subdirectory`: template root does not exist ({prefix})"
                    ));
                    return Ok(false);
                }
                Err(source) => {
                    return Err(CopierError::Io {
                        path: path.display().to_string(),
                        source,
                    });
                }
            };
            if meta.file_type().is_symlink() {
                self.find(format!(
                    "`_subdirectory`: template root goes through a symbolic link ({prefix})"
                ));
                return Ok(false);
            }
            if !meta.is_dir() {
                self.find(format!(
                    "`_subdirectory`: template root is not a directory ({prefix})"
                ));
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// Walks the template root: finds the answers-file template and Copier variables.
    fn scan_tree(
        &mut self,
        template_dir: &Path,
        root: &str,
        suffix: &str,
        remove: &mut Vec<RelPath>,
    ) -> Result<(), CopierError> {
        let base = if root == "." {
            template_dir.to_path_buf()
        } else {
            template_dir.join(root)
        };
        let answers = regex(&format!(
            r"^\{{\{{\s*_copier_conf\.answers_file\s*\}}\}}{}$",
            regex::escape(suffix)
        ))?;
        let rel = if root == "." {
            String::new()
        } else {
            root.to_string()
        };
        self.walk(&base, &rel, true, root == ".", &answers, remove)
    }

    fn walk(
        &mut self,
        dir: &Path,
        rel_dir: &str,
        at_root: bool,
        root_is_template_dir: bool,
        answers: &Regex,
        remove: &mut Vec<RelPath>,
    ) -> Result<(), CopierError> {
        let io = |path: &Path, source| CopierError::Io {
            path: path.display().to_string(),
            source,
        };
        let mut names = Vec::new();
        for entry in std::fs::read_dir(dir).map_err(|e| io(dir, e))? {
            let entry = entry.map_err(|e| io(dir, e))?;
            names.push(entry.file_name());
        }
        names.sort();
        for os_name in names {
            let name = os_name.to_string_lossy().into_owned();
            if os_name.to_str().is_none() {
                let rel = if rel_dir.is_empty() {
                    name.clone()
                } else {
                    format!("{rel_dir}/{name}")
                };
                self.find(format!("`{rel}`: file name is not valid UTF-8"));
                continue;
            }
            if name == ".git" {
                continue;
            }
            if at_root && root_is_template_dir && (name == "copier.yml" || name == "copier.yaml") {
                continue;
            }
            let path = dir.join(&os_name);
            let rel = if rel_dir.is_empty() {
                name.clone()
            } else {
                format!("{rel_dir}/{name}")
            };
            let place = format!("`{rel}`");
            let meta = std::fs::symlink_metadata(&path).map_err(|e| io(&path, e))?;
            let file_type = meta.file_type();
            if file_type.is_symlink() {
                self.find(format!(
                    "{place}: symbolic link not supported (Copier follows it, rootstock keeps it)"
                ));
                continue;
            }
            if file_type.is_file() && at_root && answers.is_match(&name) {
                remove.push(rel);
                continue;
            }
            if !file_type.is_dir() && !file_type.is_file() {
                self.find(format!(
                    "{place}: not a regular file, directory or symbolic link"
                ));
                continue;
            }
            self.check_vars(&place, &name);
            if file_type.is_dir() {
                self.walk(&path, &rel, false, root_is_template_dir, answers, remove)?;
            } else if file_type.is_file() {
                let bytes = std::fs::read(&path).map_err(|e| io(&path, e))?;
                if let Ok(text) = String::from_utf8(bytes) {
                    self.check_vars(&place, &text);
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{self, JinjaOptions, Phase, QuestionKind};
    use tempfile::TempDir;

    fn ok(yaml: &str) -> Converted {
        let dir = TempDir::new().unwrap();
        for root in ["template", "t"] {
            std::fs::create_dir(dir.path().join(root)).unwrap();
        }
        convert(yaml, dir.path()).unwrap()
    }

    fn found_in(yaml: &str, dir: &Path) -> Vec<String> {
        match convert(yaml, dir) {
            Err(CopierError::Unsupported(f)) => f.iter().map(ToString::to_string).collect(),
            other => panic!("expected findings, got {other:?}"),
        }
    }

    fn found(yaml: &str) -> Vec<String> {
        let dir = TempDir::new().unwrap();
        found_in(yaml, dir.path())
    }

    fn has(findings: &[String], needle: &str) {
        assert!(
            findings.iter().any(|l| l.contains(needle)),
            "no finding with {needle:?} in {findings:#?}"
        );
    }

    // ---- group 1: keys of the template ----

    #[test]
    fn harness_keys_are_converted() {
        let c = ok(r#"
_min_copier_version: "9.0"
_subdirectory: template
_answers_file: .copier-answers.yml
_templates_suffix: .jinja
_envops:
  keep_trailing_newline: true
  trim_blocks: true
  lstrip_blocks: true
"#);
        assert_eq!(c.config.root, "template");
        assert_eq!(c.config.suffix, ".jinja");
        assert!(c.config.jinja.trim_blocks);
        assert!(c.config.jinja.lstrip_blocks);
        assert!(c.config.jinja.keep_trailing_newline);
        assert_eq!(c.config.min_rootstock, None);
    }

    #[test]
    fn copier_defaults_for_envops() {
        let c = ok("_subdirectory: t\n");
        assert_eq!(c.config.jinja, JinjaOptions::default());
        assert!(c.text.contains("keep_trailing_newline = true"));
        assert!(c.text.contains("trim_blocks = false"));
        let c = ok("_envops:\n  keep_trailing_newline: false\n");
        assert!(!c.config.jinja.keep_trailing_newline);
    }

    #[test]
    fn default_exclude_only_without_subdirectory_and_exclude() {
        let c = ok("{}");
        assert_eq!(c.config.files.exclude, DEFAULT_EXCLUDE);
        assert!(c.config.files.exclude.contains(&"copier.yml".to_string()));
        assert!(c.config.files.exclude.contains(&".git".to_string()));
        let c = ok("_exclude: [a, b]\n");
        assert_eq!(c.config.files.exclude, ["a", "b"]);
        let c = ok("_subdirectory: template\n");
        assert!(c.config.files.exclude.is_empty());
    }

    #[test]
    fn findings_accumulate() {
        let f = found(
            r#"
_jinja_extensions: [x]
_migrations: []
_preserve_symlinks: true
_unknown: 1
"#,
        );
        assert_eq!(f.len(), 4, "{f:#?}");
        for key in [
            "_jinja_extensions",
            "_migrations",
            "_preserve_symlinks",
            "_unknown",
        ] {
            has(&f, key);
        }
    }

    #[test]
    fn envops_findings() {
        let f = found("_envops:\n  autoescape: true\n  trim_blocks: yes_please\n");
        has(&f, "autoescape");
        has(&f, "trim_blocks");
    }

    #[test]
    fn jinja_and_empty_suffix_findings() {
        let f = found("_subdirectory: \"{{ x }}\"\n_templates_suffix: \"{{ y }}\"\n");
        has(&f, "_subdirectory");
        has(&f, "_templates_suffix");
        let f = found("_templates_suffix: \"\"\n");
        has(&f, "_templates_suffix");
    }

    #[test]
    fn yaml_tag_is_a_finding() {
        let f = found("q:\n  type: str\n  default: !custom 1\n");
        has(&f, "tag");
    }

    #[test]
    fn output_starts_with_comment_and_parses() {
        let c = ok("_subdirectory: template\nq:\n  type: bool\n  default: true\n");
        assert!(
            c.text
                .starts_with("# Converted from copier.yml by rootstock import-copier.\n")
        );
        let parsed = config::parse(&c.text, Path::new("rootstock.toml")).unwrap();
        assert_eq!(parsed, c.config);
    }

    #[test]
    fn not_a_mapping_and_multiple_documents_are_errors() {
        let dir = TempDir::new().unwrap();
        assert!(matches!(
            convert("- a\n", dir.path()),
            Err(CopierError::NotMapping)
        ));
        assert!(matches!(
            convert("a: 1\n---\nb: 2\n", dir.path()),
            Err(CopierError::Yaml(_))
        ));
    }

    // ---- group 2: rules and tasks ----

    #[test]
    fn skip_if_exists_becomes_starter_verbatim() {
        let c = ok(r#"
_skip_if_exists:
  - /README.md
  - "{% if spec_path %}{{ spec_path }}/**{% endif %}"
"#);
        assert_eq!(
            c.config.files.starter,
            [
                "/README.md",
                "{% if spec_path %}{{ spec_path }}/**{% endif %}"
            ]
        );
    }

    #[test]
    fn update_only_excludes_with_a_pair_are_dropped() {
        let c = ok(r#"
_exclude:
  - _partials
  - "{% if _copier_operation == 'update' %}/README.md{% endif %}"
  - "{%if _copier_operation==\"update\" and spec_path%}{{ spec_path }}/**{% endif %}"
_skip_if_exists:
  - /README.md
  - "{% if spec_path %}{{ spec_path }}/**{% endif %}"
"#);
        assert_eq!(c.config.files.exclude, ["_partials"]);
        assert_eq!(c.config.files.starter.len(), 2);
    }

    #[test]
    fn update_only_exclude_without_a_pair_is_a_finding() {
        let f =
            found("_exclude:\n  - \"{% if _copier_operation == 'update' %}/a.txt{% endif %}\"\n");
        has(&f, "matching `_skip_if_exists`");
        let f = found(
            r#"
_exclude:
  - "{% if _copier_operation == 'update' and c %}p{% endif %}"
_skip_if_exists:
  - p
"#,
        );
        has(&f, "matching `_skip_if_exists`");
    }

    #[test]
    fn copier_operation_elsewhere_is_a_finding() {
        let f = found("_exclude:\n  - \"{% if _copier_operation == 'copy' %}x{% endif %}\"\n");
        has(&f, "_copier_operation");
        let f = found("_skip_if_exists:\n  - \"{{ _copier_operation }}\"\n");
        has(&f, "_copier_operation");
    }

    #[test]
    fn tasks_map_to_hook_phases() {
        let c = ok(r#"
_tasks:
  - command: a
    when: "{{ _copier_operation == 'copy' }}"
  - command: b
    when: "{{_copier_operation=='update'}}"
  - echo c
  - command: d
  - command: e
    when: true
"#);
        let hooks = &c.config.hooks;
        assert_eq!(hooks.len(), 5);
        assert_eq!(hooks[0].run, "a");
        assert_eq!(hooks[0].when, [Phase::New]);
        assert_eq!(hooks[1].when, [Phase::Update]);
        assert_eq!(hooks[2].run, "echo c");
        assert_eq!(hooks[2].when, [Phase::New, Phase::Update]);
        assert_eq!(hooks[3].when, [Phase::New, Phase::Update]);
        assert_eq!(hooks[4].when, [Phase::New, Phase::Update]);
    }

    #[test]
    fn task_findings() {
        let f = found("_tasks:\n  - command: a\n    when: \"{{ x }}\"\n");
        has(&f, "_tasks");
        let f = found("_tasks:\n  - command: [a, b]\n");
        has(&f, "_tasks");
        let f = found("_tasks:\n  - command: a\n    working_directory: x\n");
        has(&f, "working_directory");
        let f = found("_tasks:\n  - command: \"echo {{ project_name }}\"\n");
        has(&f, "Jinja");
    }

    // ---- group 3: questions ----

    #[test]
    fn simple_questions_keep_their_fields_and_order() {
        let c = ok(r#"
b:
  type: bool
  default: true
  help: Is it?
a:
  type: str
  default: x
  validator: "{% if not a %}bad{% endif %}"
c:
  type: yaml
  default:
    k: v
    list: [1, 2]
"#);
        let q = &c.config.questions;
        assert_eq!(
            q.iter().map(|q| q.name.as_str()).collect::<Vec<_>>(),
            ["b", "a", "c"]
        );
        assert_eq!(q[0].kind, QuestionKind::Bool);
        assert_eq!(q[0].help.as_deref(), Some("Is it?"));
        assert_eq!(q[0].default, Some(toml::Value::Boolean(true)));
        assert_eq!(q[1].kind, QuestionKind::Str);
        assert_eq!(
            q[1].validator.as_deref(),
            Some("{% if not a %}bad{% endif %}")
        );
        assert_eq!(q[2].kind, QuestionKind::Yaml);
        let table = q[2].default.as_ref().and_then(|v| v.as_table()).unwrap();
        assert_eq!(table["k"].as_str(), Some("v"));
        assert_eq!(table["list"].as_array().map(Vec::len), Some(2));
    }

    #[test]
    fn str_with_choices_becomes_choice() {
        let c = ok(r#"
doc_form:
  type: str
  default: catalog
  choices:
    "catalog — dir": catalog
    "monolith — one file": monolith
plain:
  type: str
  choices: [a, b]
"#);
        let q = &c.config.questions;
        assert_eq!(q[0].kind, QuestionKind::Choice);
        assert_eq!(q[0].choices.len(), 2);
        assert_eq!(q[0].choices[0].label, "catalog — dir");
        assert_eq!(q[0].choices[0].value.as_str(), Some("catalog"));
        assert_eq!(
            q[0].default.as_ref().and_then(|v| v.as_str()),
            Some("catalog")
        );
        assert_eq!(q[1].kind, QuestionKind::Choice);
        assert_eq!(q[1].choices[1].label, "b");
        assert_eq!(q[1].choices[1].value.as_str(), Some("b"));
    }

    #[test]
    fn when_is_unwrapped() {
        let c = ok(r#"
a:
  type: bool
  when: "{{ not (rust and frontend) }}"
b:
  type: bool
  when: false
c:
  type: bool
  when: "{{- x -}}"
"#);
        let q = &c.config.questions;
        assert_eq!(q[0].when.as_deref(), Some("not (rust and frontend)"));
        assert_eq!(q[1].when.as_deref(), Some("false"));
        assert_eq!(q[2].when.as_deref(), Some("x"));
    }

    #[test]
    fn question_findings() {
        let f = found("q:\n  default: 1\n");
        has(&f, "question `q`");
        let f = found("q:\n  type: int\n");
        has(&f, "`int`");
        let f = found("q:\n  type: str\n  secret: true\n");
        has(&f, "secret");
        let f = found("q: just a value\n");
        has(&f, "question `q`");
        let f = found("q:\n  type: bool\n  choices: [a]\n");
        has(&f, "choices");
        let f = found("q:\n  type: str\n  choices: [[a], b]\n");
        has(&f, "choices");
        let f = found("q:\n  type: yaml\n  default:\n    a: null\n");
        has(&f, "null");
        let f = found("q:\n  type: bool\n  when: \"{% if x %}1{% endif %}\"\n");
        has(&f, "when");
        let f = found("q:\n  type: bool\n  when: \"{{ a }} and {{ b }}\"\n");
        has(&f, "when");
        let f = found("q:\n  type: str\n  default: \"{{ _copier_answers }}\"\n");
        has(&f, "_copier_answers");
    }

    // ---- group 4: the template tree ----

    fn tree(files: &[(&str, &str)]) -> TempDir {
        let dir = TempDir::new().unwrap();
        for (path, content) in files {
            let full = dir.path().join(path);
            std::fs::create_dir_all(full.parent().unwrap()).unwrap();
            std::fs::write(full, content).unwrap();
        }
        dir
    }

    const TREE_YAML: &str = "_subdirectory: template\n_templates_suffix: .jinja\n";

    #[test]
    fn answers_file_templates_in_the_root_are_removed() {
        let dir = tree(&[
            ("template/{{ _copier_conf.answers_file }}.jinja", "x"),
            ("template/{{_copier_conf.answers_file}}.jinja", "x"),
            ("template/keep.txt", "ok"),
        ]);
        let c = convert(TREE_YAML, dir.path()).unwrap();
        let mut remove = c.remove.clone();
        remove.sort();
        assert_eq!(
            remove,
            [
                "template/{{ _copier_conf.answers_file }}.jinja",
                "template/{{_copier_conf.answers_file}}.jinja"
            ]
        );
    }

    #[test]
    fn nested_answers_file_is_a_finding() {
        let dir = tree(&[("template/sub/{{ _copier_conf.answers_file }}.jinja", "x")]);
        let f = found_in(TREE_YAML, dir.path());
        has(&f, "template/sub/");
    }

    #[test]
    fn copier_variables_in_files_and_names_are_findings() {
        let dir = tree(&[
            ("template/a.txt.jinja", "{{ _copier_conf.src_path }}"),
            ("template/b.txt", "uses _copier_answers"),
            ("template/{{ _copier_conf.x }}/f.txt", "plain"),
        ]);
        let f = found_in(TREE_YAML, dir.path());
        has(&f, "template/a.txt.jinja");
        has(&f, "template/b.txt");
        has(&f, "template/{{ _copier_conf.x }}");
    }

    #[cfg(unix)]
    #[test]
    fn symlink_is_a_finding() {
        let dir = tree(&[("template/real.txt", "x")]);
        std::os::unix::fs::symlink("real.txt", dir.path().join("template/link.txt")).unwrap();
        let f = found_in(TREE_YAML, dir.path());
        has(&f, "template/link.txt");
        has(&f, "symbolic link");
    }

    #[test]
    fn non_utf8_file_is_not_scanned() {
        let dir = TempDir::new().unwrap();
        std::fs::create_dir_all(dir.path().join("template")).unwrap();
        let mut bytes = b"_copier_conf ".to_vec();
        bytes.extend_from_slice(&[0xff, 0xfe, 0x00]);
        std::fs::write(dir.path().join("template/bin.dat"), bytes).unwrap();
        assert!(convert(TREE_YAML, dir.path()).is_ok());
    }

    #[test]
    fn git_directory_is_not_walked() {
        let dir = tree(&[("template/.git/config", "_copier_conf")]);
        assert!(convert(TREE_YAML, dir.path()).is_ok());
    }

    #[test]
    fn bad_roots_are_findings() {
        for root in ["../outside", "/abs", "a/../../b", "a:b", "a\\b"] {
            let dir = tree(&[("keep.txt", "x")]);
            let f = found_in(&format!("_subdirectory: '{root}'\n"), dir.path());
            has(&f, "must be a relative path inside the template");
        }
    }

    #[test]
    fn missing_and_file_roots_are_findings() {
        let dir = tree(&[("file", "x")]);
        let f = found_in("_subdirectory: nope\n", dir.path());
        has(&f, "template root does not exist");
        let f = found_in("_subdirectory: file\n", dir.path());
        has(&f, "template root is not a directory");
        let f = found_in("_subdirectory: file/inner\n", dir.path());
        has(&f, "template root is not a directory");
    }

    #[cfg(unix)]
    #[test]
    fn root_through_a_symlink_is_a_finding_and_nothing_is_scanned() {
        let outside = tree(&[("b/{{ _copier_conf.answers_file }}.jinja", "x")]);
        let dir = TempDir::new().unwrap();
        std::os::unix::fs::symlink(outside.path(), dir.path().join("a")).unwrap();
        let f = found_in("_subdirectory: a/b\n", dir.path());
        has(&f, "goes through a symbolic link");
        let f = found_in("_subdirectory: a\n", dir.path());
        has(&f, "goes through a symbolic link");
    }

    #[test]
    fn root_is_normalized() {
        for sub in ["\"\"", ".", "./", "./."] {
            let dir = tree(&[
                ("copier.yml", "{{ _copier_operation }}"),
                ("{{ _copier_conf.answers_file }}.jinja", "x"),
            ]);
            let c = convert(&format!("_subdirectory: {sub}\n"), dir.path()).unwrap();
            assert_eq!(c.config.root, ".", "{sub}");
            assert_eq!(c.config.files.exclude, DEFAULT_EXCLUDE, "{sub}");
            assert_eq!(c.remove, ["{{ _copier_conf.answers_file }}.jinja"], "{sub}");
        }
        let dir = tree(&[("template/{{ _copier_conf.answers_file }}.jinja", "x")]);
        let c = convert("_subdirectory: ./template/\n", dir.path()).unwrap();
        assert_eq!(c.config.root, "template");
        assert_eq!(c.remove, ["template/{{ _copier_conf.answers_file }}.jinja"]);
    }

    #[test]
    fn answers_file_follows_a_custom_suffix() {
        let dir = tree(&[
            ("template/{{ _copier_conf.answers_file }}.tmpl", "x"),
            ("template/{{ _copier_conf.answers_file }}.jinja", "x"),
        ]);
        let f = found_in(
            "_subdirectory: template\n_templates_suffix: .tmpl\n",
            dir.path(),
        );
        // `.jinja` is an ordinary file for this suffix, and it mentions a variable.
        has(&f, "template/{{ _copier_conf.answers_file }}.jinja");
        assert!(!f.iter().any(|l| l.contains(".tmpl")), "{f:#?}");
    }

    #[test]
    fn folder_name_and_external_data_are_findings() {
        let f = found("q:\n  type: str\n  default: \"{{ _folder_name }}\"\n");
        has(&f, "_folder_name");
        let f = found("q:\n  type: str\n  help: \"{{ _external_data.x }}\"\n");
        has(&f, "_external_data");
    }

    #[test]
    fn update_only_forms_with_extra_logic_are_not_recognized() {
        let f = found(
            r#"
_exclude:
  - "{% if _copier_operation == 'update' and a or b %}p{% endif %}"
_skip_if_exists:
  - "{% if a or b %}p{% endif %}"
"#,
        );
        has(&f, "_copier_operation");
        let f = found(
            r#"
_exclude:
  - "{% if _copier_operation == 'update' %}R{% else %}S{% endif %}"
_skip_if_exists:
  - "R{% else %}S"
"#,
        );
        has(&f, "_copier_operation");
    }

    #[test]
    fn findings_in_rules_name_the_entry_index() {
        let f = found(
            "_exclude:\n  - a\n  - \"{{ _copier_operation }}\"\n  - \"{{ _copier_operation }}\"\n_skip_if_exists:\n  - ok\n  - \"{{ _copier_operation }}\"\n",
        );
        has(&f, "`_exclude[1]`");
        has(&f, "`_exclude[2]`");
        has(&f, "`_skip_if_exists[1]`");
    }

    #[test]
    fn jinja_comment_in_a_command_is_a_finding() {
        let f = found("_tasks:\n  - \"echo {# x #}\"\n");
        has(&f, "Jinja");
    }

    #[cfg(unix)]
    #[test]
    fn fifo_is_a_finding() {
        let dir = tree(&[("template/a.txt", "x")]);
        let fifo = dir.path().join("template/pipe");
        let made = std::process::Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .is_ok_and(|s| s.success());
        if !made {
            eprintln!("skipped: mkfifo is not available");
            return;
        }
        let f = found_in(TREE_YAML, dir.path());
        has(&f, "template/pipe");
    }

    #[cfg(unix)]
    #[test]
    fn non_utf8_name_is_a_finding() {
        use std::os::unix::ffi::OsStrExt;
        let dir = tree(&[("template/a.txt", "x")]);
        let name = std::ffi::OsStr::from_bytes(b"bad-\xff-name");
        if std::fs::write(dir.path().join("template").join(name), "x").is_err() {
            eprintln!("skipped: the file system rejects non-UTF-8 names");
            return;
        }
        let f = found_in(TREE_YAML, dir.path());
        has(&f, "not valid UTF-8");
    }

    #[test]
    fn harness_copier_yml_converts_without_findings() {
        let yaml = include_str!("../tests/fixtures/copier/harness-v0.4.5.yml");
        let c = ok(yaml);
        assert_eq!(c.config.root, "template");
        assert_eq!(c.config.files.exclude, ["_partials"]);
        assert!(c.config.files.starter.iter().any(|s| s == "/README.md"));
        assert_eq!(c.config.hooks.len(), 1);
        assert_eq!(c.config.hooks[0].when, [Phase::New]);
        assert_eq!(c.config.questions[0].name, "project_name");
        let doc_form = c
            .config
            .questions
            .iter()
            .find(|q| q.name == "doc_form")
            .unwrap();
        assert_eq!(doc_form.kind, QuestionKind::Choice);
        assert_eq!(doc_form.choices.len(), 2);
    }
}
