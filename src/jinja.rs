//! Jinja environment shared by answers, file contents and file names.

use std::path::Path;

use minijinja::{
    AutoEscape, Environment, Error, ErrorKind, TemplateSource, Value, syntax::SyntaxConfig,
    value::Serde,
};

use crate::config::JinjaOptions;
use crate::record::Answers;

/// A template or expression failed to compile or render.
#[derive(Debug, thiserror::Error)]
#[error("{}", describe(.0))]
pub struct JinjaError(#[from] minijinja::Error);

/// The error text with its template name, line and source excerpt.
fn describe(e: &Error) -> String {
    let mut out = e.to_string();
    let mut source = std::error::Error::source(e);
    while let Some(cause) = source {
        out.push_str(&format!("\ncaused by: {cause}"));
        source = cause.source();
    }
    let info = e.display_debug_info().to_string();
    if !info.is_empty() {
        out.push('\n');
        out.push_str(&info);
    }
    out
}

/// Jinja environment configured from the template's options.
pub struct Jinja {
    env: Environment<'static>,
}

impl Jinja {
    /// `include` and `import` names resolve relative to `include_root`.
    pub fn new(opts: &JinjaOptions, include_root: &Path) -> Jinja {
        let mut env = Environment::new();
        let mut syntax = SyntaxConfig::builder();
        syntax
            .trim_blocks(opts.trim_blocks)
            .lstrip_blocks(opts.lstrip_blocks)
            .keep_trailing_newline(opts.keep_trailing_newline);
        // Default delimiters cannot conflict, so build() does not fail here.
        env.set_syntax(syntax.build().unwrap_or_default());
        env.set_auto_escape_callback(|_| AutoEscape::None);
        let root = include_root.to_path_buf();
        env.set_loader(move |name| load_include(&root, name));
        env.add_filter("regex_search", regex_search);
        Jinja { env }
    }

    /// Renders `source`; `name` appears only in error messages.
    pub fn render_str(
        &self,
        name: &str,
        source: &str,
        ctx: &Answers,
    ) -> Result<String, JinjaError> {
        Ok(self
            .env
            .render_named_str(name, source, Value::from(Serde(ctx)))?)
    }

    /// Evaluates a Jinja expression and applies Jinja truthiness.
    pub fn eval_bool(&self, expr: &str, ctx: &Answers) -> Result<bool, JinjaError> {
        let value = self
            .env
            .compile_expression(expr)?
            .eval(Value::from(Serde(ctx)))?;
        Ok(value.is_true())
    }
}

/// Loads an include by name relative to `root`, like Jinja2's FileSystemLoader:
/// `.` and empty segments are dropped, dot-prefixed names are allowed. Names
/// with `..`, a backslash or an absolute form are reported as not found.
fn load_include(root: &Path, name: &str) -> Result<Option<TemplateSource>, Error> {
    if name.contains('\\') || name.starts_with('/') {
        return Ok(None);
    }
    let mut path = root.to_path_buf();
    for seg in name.split('/') {
        match seg {
            "" | "." => {}
            ".." => return Ok(None),
            s if matches!(
                Path::new(s).components().next(),
                Some(std::path::Component::Normal(_))
            ) =>
            {
                path.push(s)
            }
            _ => return Ok(None),
        }
    }
    if path == root {
        return Ok(None);
    }
    match std::fs::read_to_string(&path) {
        Ok(src) => Ok(Some(TemplateSource::new(src))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(Error::new(
            ErrorKind::InvalidOperation,
            format!("could not read template {}", path.display()),
        )
        .with_source(e)),
    }
}

/// First match of `pattern` in `value`, or none when there is no match.
fn regex_search(value: Value, pattern: &str) -> Result<Value, Error> {
    let re = regex::Regex::new(pattern).map_err(|e| {
        Error::new(
            ErrorKind::InvalidOperation,
            format!("invalid regex pattern {pattern:?}: {e}"),
        )
    })?;
    let text = value.to_string();
    Ok(match re.find(&text) {
        Some(m) => Value::from(m.as_str()),
        None => Value::from(()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn answers(v: serde_json::Value) -> Answers {
        match v {
            serde_json::Value::Object(m) => m.into_iter().collect(),
            _ => Answers::new(),
        }
    }

    fn plain() -> Jinja {
        Jinja::new(&JinjaOptions::default(), Path::new("."))
    }

    #[test]
    fn keeps_trailing_newline_by_default() {
        let out = plain().render_str("t", "a\n", &Answers::new()).unwrap();
        assert_eq!(out, "a\n");
    }

    #[test]
    fn drops_trailing_newline_when_disabled() {
        let opts = JinjaOptions {
            keep_trailing_newline: false,
            ..JinjaOptions::default()
        };
        let j = Jinja::new(&opts, Path::new("."));
        assert_eq!(j.render_str("t", "a\n", &Answers::new()).unwrap(), "a");
    }

    #[test]
    fn trim_and_lstrip_blocks() {
        let opts = JinjaOptions {
            trim_blocks: true,
            lstrip_blocks: true,
            keep_trailing_newline: true,
        };
        let j = Jinja::new(&opts, Path::new("."));
        let out = j
            .render_str(
                "t",
                "  {% if x %}\nA\n  {% endif %}\n",
                &answers(json!({"x": true})),
            )
            .unwrap();
        assert_eq!(out, "A\n");
    }

    #[test]
    fn renders_answers() {
        let out = plain()
            .render_str(
                "t",
                "{{ n }}-{{ xs[1] }}",
                &answers(json!({"n": "p", "xs": [1, 2]})),
            )
            .unwrap();
        assert_eq!(out, "p-2");
    }

    #[test]
    fn regex_search_match_and_miss() {
        let j = plain();
        let c = Answers::new();
        assert_eq!(
            j.render_str("t", "{{ 'ab-1' | regex_search('^[a-z]+') }}", &c)
                .unwrap(),
            "ab"
        );
        assert_eq!(
            j.render_str(
                "t",
                "{% if 'AB' | regex_search('^[a-z]+') %}y{% else %}n{% endif %}",
                &c
            )
            .unwrap(),
            "n"
        );
    }

    #[test]
    fn regex_search_invalid_pattern_names_it() {
        let err = plain()
            .render_str("t", "{{ 'a' | regex_search('(') }}", &Answers::new())
            .unwrap_err();
        assert!(
            err.to_string().contains("'('") || err.to_string().contains("\"(\""),
            "{err}"
        );
    }

    #[test]
    fn no_autoescape_for_html() {
        let out = plain()
            .render_str(
                "index.html.jinja",
                "{{ v }}",
                &answers(json!({"v": "<b>&"})),
            )
            .unwrap();
        assert_eq!(out, "<b>&");
    }

    #[test]
    fn undefined_is_empty_and_falsy() {
        let j = plain();
        assert_eq!(
            j.render_str("t", "[{{ nope }}]", &Answers::new()).unwrap(),
            "[]"
        );
        assert!(!j.eval_bool("nope", &Answers::new()).unwrap());
    }

    #[test]
    fn include_resolves_from_include_root() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("template/_partials");
        std::fs::create_dir_all(&p).unwrap();
        std::fs::write(p.join("x.md.jinja"), "hi {{ who }}").unwrap();
        let j = Jinja::new(&JinjaOptions::default(), dir.path());
        let out = j
            .render_str(
                "t",
                "{% include 'template/_partials/x.md.jinja' %}!",
                &answers(json!({"who": "you"})),
            )
            .unwrap();
        assert_eq!(out, "hi you!");
    }

    #[test]
    fn include_cannot_escape_root() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("root");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(dir.path().join("secret.txt"), "SECRET").unwrap();
        let abs = dir.path().join("secret.txt");
        let j = Jinja::new(&JinjaOptions::default(), &root);
        let c = Answers::new();
        let not_found = |r: Result<String, JinjaError>| {
            r.expect_err("must not load").0.kind() == ErrorKind::TemplateNotFound
        };
        assert!(not_found(j.render_str(
            "t",
            "{% include '../secret.txt' %}",
            &c
        )));
        assert!(not_found(j.render_str(
            "t",
            "{% include 'a/../../secret.txt' %}",
            &c
        )));
        assert!(not_found(j.render_str(
            "t",
            "{% include 'a\\\\..\\\\secret.txt' %}",
            &c
        )));
        let src = format!("{{% include '{}' %}}", abs.display());
        assert!(not_found(j.render_str("t", &src, &c)));
    }

    #[test]
    fn include_from_dot_directory_and_dot_prefix() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".hidden")).unwrap();
        std::fs::write(dir.path().join(".hidden/x.jinja"), "H").unwrap();
        std::fs::write(dir.path().join("y.jinja"), "Y").unwrap();
        let j = Jinja::new(&JinjaOptions::default(), dir.path());
        let c = Answers::new();
        let src = "{% include '.hidden/x.jinja' %}{% include './y.jinja' %}";
        assert_eq!(j.render_str("t", src, &c).unwrap(), "HY");
    }

    #[test]
    fn eval_bool_uses_answers() {
        let j = plain();
        let c = answers(json!({"rust": true, "frontend": false}));
        assert!(j.eval_bool("rust and not frontend", &c).unwrap());
        assert!(!j.eval_bool("frontend", &c).unwrap());
    }

    #[test]
    fn errors_carry_template_name_and_line() {
        let err = plain()
            .render_str("broken.md.jinja", "ok\n{% if %}\n", &Answers::new())
            .unwrap_err();
        let s = err.to_string();
        assert!(s.contains("broken.md.jinja"), "{s}");
        assert!(s.contains("line 2") || s.contains(":2"), "{s}");
    }
}
