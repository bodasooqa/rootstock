//! Jinja environment shared by answers, file contents and file names.

use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use minijinja::{
    AutoEscape, Environment, Error, ErrorKind, State, TemplateSource, Value,
    syntax::SyntaxConfig,
    value::{Enumerator, Object, ObjectRepr, Serde, ValueKind},
};
use minijinja_contrib::pycompat;

use crate::config::JinjaOptions;
use crate::record::Answers;

/// A template or expression failed to compile or render.
///
/// The Display text already holds minijinja's causes and source excerpt, so
/// the inner error is deliberately not exposed as `source`: a `{:#}` chain
/// would print it twice.
#[derive(Debug, thiserror::Error)]
#[error("{}", describe(.0))]
pub struct JinjaError(minijinja::Error);

impl From<minijinja::Error> for JinjaError {
    fn from(e: minijinja::Error) -> Self {
        JinjaError(e)
    }
}

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
    /// `include` and `import` names resolve relative to `template_dir`, the directory holding `rootstock.toml`.
    pub fn new(opts: &JinjaOptions, template_dir: &Path) -> Jinja {
        let mut env = Environment::new();
        let mut syntax = SyntaxConfig::builder();
        syntax
            .trim_blocks(opts.trim_blocks)
            .lstrip_blocks(opts.lstrip_blocks)
            .keep_trailing_newline(opts.keep_trailing_newline);
        // Default delimiters cannot conflict, so build() does not fail here.
        env.set_syntax(syntax.build().unwrap_or_default());
        env.set_auto_escape_callback(|_| AutoEscape::None);
        // Includes may not reach outside the canonical template directory, even
        // through symlinks; a missing root means every include is not found.
        let root = std::fs::canonicalize(template_dir).ok();
        env.set_loader(move |name| match &root {
            Some(root) => load_include(root, name),
            None => Ok(None),
        });
        env.add_filter("regex_search", regex_search);
        env.set_unknown_method_callback(unknown_method);
        env.add_function(LIST_FN, || Value::from_object(MutableList::default()));
        Jinja { env }
    }

    /// Renders `source`; `name` appears only in error messages.
    pub fn render_str(
        &self,
        name: &str,
        source: &str,
        ctx: &Answers,
    ) -> Result<String, JinjaError> {
        let source = rewrite_list_literals(source);
        Ok(self
            .env
            .render_named_str(name, &source, Value::from(Serde(ctx)))?)
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
    let Ok(real) = std::fs::canonicalize(&path) else {
        return Ok(None);
    };
    if !real.starts_with(root) {
        return Ok(None);
    }
    match std::fs::read_to_string(&real) {
        Ok(src) => Ok(Some(TemplateSource::new(rewrite_list_literals(&src)))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(Error::new(
            ErrorKind::InvalidOperation,
            format!("could not read template {}", path.display()),
        )
        .with_source(e)),
    }
}

/// Name of the global function that creates a [`MutableList`].
const LIST_FN: &str = "__rootstock_list";

/// Hint attached to `append` on a list that is not a [`MutableList`].
const APPEND_HINT: &str = "`append` works only on a list created with `{% set name = [] %}`";

/// A list that `append` changes in place, as in Python.
#[derive(Debug, Default)]
struct MutableList(Mutex<Vec<Value>>);

/// Locks the list; a poisoned lock still holds intact data (only `push` runs under it).
fn items(list: &MutableList) -> MutexGuard<'_, Vec<Value>> {
    list.0.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Object for MutableList {
    fn repr(self: &Arc<Self>) -> ObjectRepr {
        ObjectRepr::Seq
    }

    fn get_value(self: &Arc<Self>, key: &Value) -> Option<Value> {
        items(self).get(key.as_usize()?).cloned()
    }

    fn enumerate(self: &Arc<Self>) -> Enumerator {
        Enumerator::Seq(items(self).len())
    }

    fn call_method(
        self: &Arc<Self>,
        _state: &mut State<'_, '_>,
        method: &str,
        args: &[Value],
    ) -> Result<Value, Error> {
        match (method, args) {
            ("append", [item]) => {
                items(self).push(item.clone());
                Ok(Value::from(()))
            }
            ("append", _) => Err(Error::new(
                ErrorKind::InvalidOperation,
                "`append` takes exactly one argument",
            )),
            // Other names go to the unknown-method callback.
            _ => Err(Error::from(ErrorKind::UnknownMethod)),
        }
    }
}

/// Python methods from `pycompat`, plus a hint when `append` hits a plain list.
fn unknown_method(
    state: &mut State<'_, '_>,
    value: &Value,
    method: &str,
    args: &[Value],
) -> Result<Value, Error> {
    match pycompat::unknown_method_callback(state, value, method, args) {
        Err(e)
            if e.kind() == ErrorKind::UnknownMethod
                && method == "append"
                && value.kind() == ValueKind::Seq
                && value.downcast_object_ref::<MutableList>().is_none() =>
        {
            Err(Error::new(ErrorKind::InvalidOperation, APPEND_HINT))
        }
        other => other,
    }
}

/// End of the string literal that starts at `start` (a quote), exclusive.
fn skip_string(b: &[u8], start: usize) -> usize {
    let Some(&quote) = b.get(start) else {
        return b.len();
    };
    let mut i = start + 1;
    while i < b.len() {
        match b[i] {
            b'\\' => i += 2,
            c if c == quote => return i + 1,
            _ => i += 1,
        }
    }
    b.len()
}

/// Index just past the `%}` that ends a block tag whose body starts at
/// `from`, skipping quoted strings; `None` if the tag is not closed.
fn block_end(b: &[u8], from: usize) -> Option<usize> {
    let mut i = from;
    while i < b.len() {
        match b[i] {
            b'\'' | b'"' => i = skip_string(b, i),
            _ if b[i..].starts_with(b"%}") => return Some(i + 2),
            _ => i += 1,
        }
    }
    None
}

/// Index just past the `}}` that ends an expression whose body starts at
/// `from`: closes only at bracket depth zero outside strings, as in the
/// minijinja lexer.
fn expr_end(b: &[u8], from: usize) -> Option<usize> {
    let mut depth = 0usize;
    let mut i = from;
    while i < b.len() {
        match b[i] {
            b'\'' | b'"' => i = skip_string(b, i),
            b'(' | b'[' | b'{' => {
                depth += 1;
                i += 1;
            }
            b')' | b']' | b'}' if depth > 0 => {
                depth -= 1;
                i += 1;
            }
            b'}' if b[i..].starts_with(b"}}") => return Some(i + 2),
            _ => i += 1,
        }
    }
    None
}

/// Index just past `pat` at or after `from`.
fn find_after(b: &[u8], from: usize, pat: &[u8]) -> Option<usize> {
    let tail = b.get(from..)?;
    let at = tail.windows(pat.len()).position(|w| w == pat)?;
    Some(from + at + pat.len())
}

/// Index just past the `{% endraw %}` tag found at or after `from`, matched
/// lexically: no quotes or other tags are interpreted inside `raw`.
fn raw_end(b: &[u8], from: usize) -> Option<usize> {
    let mut j = from;
    loop {
        let open = find_after(b, j, b"{%")?;
        let mut k = open;
        let skip_ws = |mut k: usize| {
            while b.get(k).is_some_and(u8::is_ascii_whitespace) {
                k += 1;
            }
            k
        };
        let skip_marker = |k: usize| k + usize::from(matches!(b.get(k), Some(b'-' | b'+')));
        k = skip_ws(skip_marker(k));
        if b.get(k..).is_some_and(|t| t.starts_with(b"endraw")) {
            k = skip_marker(skip_ws(k + "endraw".len()));
            if b.get(k..).is_some_and(|t| t.starts_with(b"%}")) {
                return Some(k + 2);
            }
        }
        j = open;
    }
}

/// Body of a block tag `{% … %}` without delimiters and `-`/`+` markers,
/// and the offset of the body within the tag.
fn tag_body(tag: &str) -> Option<(usize, &str)> {
    let mut body = tag.get(2..tag.len().checked_sub(2)?)?;
    let mut start = 2;
    if let Some(rest) = body.strip_prefix(['-', '+']) {
        body = rest;
        start += 1;
    }
    if let Some(rest) = body.strip_suffix(['-', '+']) {
        body = rest;
    }
    Some((start, body))
}

/// If `body` is `set <name> = []` (spaces allowed inside the brackets), the
/// byte range of the brackets within it.
fn set_empty_list(body: &str) -> Option<(usize, usize)> {
    let trimmed = body.trim_end();
    let open = trimmed.strip_suffix(']')?.trim_end().strip_suffix('[')?;
    let head = open.trim_end().strip_suffix('=')?;
    let after_set = head.trim_start().strip_prefix("set")?;
    if !after_set.starts_with(char::is_whitespace) {
        return None;
    }
    let name = after_set.trim();
    let valid = !name.is_empty()
        && !name.starts_with(|c: char| c.is_ascii_digit())
        && name.chars().all(|c| c.is_alphanumeric() || c == '_');
    valid.then_some((open.len(), trimmed.len()))
}

/// Replaces `[]` with a call to the mutable-list function in every
/// `{% set name = [] %}` tag. Comments, expressions, `raw` blocks and strings
/// stay as they are; text without that form comes back unchanged.
fn rewrite_list_literals(source: &str) -> String {
    scan_and_rewrite(source).unwrap_or_else(|| source.to_string())
}

/// The scanner works on bytes: every delimiter is ASCII, so each slice below
/// starts and ends on a character boundary. `get` guards that anyway, and a
/// failure leaves the source untouched.
fn scan_and_rewrite(source: &str) -> Option<String> {
    let b = source.as_bytes();
    let mut out = String::with_capacity(source.len());
    let mut i = 0;
    while let Some(off) = b.get(i..)?.iter().position(|&c| c == b'{') {
        let at = i + off;
        let end = match b.get(at + 1) {
            Some(b'#') => find_after(b, at + 2, b"#}"),
            Some(b'{') => expr_end(b, at + 2),
            Some(b'%') => block_end(b, at + 2),
            _ => {
                out.push_str(source.get(i..=at)?);
                i = at + 1;
                continue;
            }
        };
        // An unclosed tag ends the scan; the rest is copied as is.
        let Some(end) = end else {
            break;
        };
        out.push_str(source.get(i..at)?);
        let tag = source.get(at..end)?;
        i = end;
        if b[at + 1] != b'%' {
            out.push_str(tag);
            continue;
        }
        let (start, body) = tag_body(tag)?;
        if body.trim() == "raw" {
            let stop = raw_end(b, end).unwrap_or(b.len());
            out.push_str(source.get(at..stop)?);
            i = stop;
        } else if let Some((from, to)) = set_empty_list(body) {
            out.push_str(tag.get(..start + from)?);
            out.push_str(LIST_FN);
            out.push_str("()");
            out.push_str(tag.get(start + to..)?);
        } else {
            out.push_str(tag);
        }
    }
    out.push_str(source.get(i..)?);
    Some(out)
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

    #[cfg(unix)]
    #[test]
    fn include_symlinks_stay_inside_root() {
        let d = tempfile::tempdir().unwrap();
        let root = d.path().join("inc");
        std::fs::create_dir(&root).unwrap();
        std::fs::write(d.path().join("secret"), "SECRET").unwrap();
        std::fs::write(root.join("real"), "ok").unwrap();
        std::os::unix::fs::symlink("../secret", root.join("out")).unwrap();
        std::os::unix::fs::symlink("real", root.join("in")).unwrap();
        let j = Jinja::new(&JinjaOptions::default(), &root);
        let r = |t| j.render_str("t", t, &Answers::new());
        assert!(r("{% include 'out' %}").is_err());
        assert_eq!(r("{% include 'in' %}").unwrap(), "ok");
        let j = Jinja::new(&JinjaOptions::default(), &d.path().join("missing"));
        assert!(
            j.render_str("t", "{% include 'x' %}", &Answers::new())
                .is_err()
        );
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
    fn include_resolves_from_template_dir() {
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

    fn render(t: &str, a: serde_json::Value) -> Result<String, JinjaError> {
        plain().render_str("t", t, &answers(a))
    }

    #[test]
    fn string_methods() {
        let out = render(
            "{{ p[:-3] if p.endswith('.md') else p }}|{{ 'Ab'.lower() }}",
            json!({"p": "k/oq.md"}),
        )
        .unwrap();
        assert_eq!(out, "k/oq|ab");
    }

    #[test]
    fn dict_methods() {
        let out = render(
            "{{ d.items() | list | length }}|{{ d.get('x', 1) }}|{{ d.keys() | list }}",
            json!({"d": {"a": 1}}),
        )
        .unwrap();
        assert_eq!(out, "1|1|['a']");
    }

    #[test]
    fn unknown_method_is_an_error() {
        assert!(render("{{ 'a'.nope() }}", json!({})).is_err());
    }

    const PI_TOOLS: &str = r#"{% set pi_tool_names = {"Read": ["read"], "Glob": ["find", "ls"]} %}{% macro pi_tools(tools) %}{% set names = [] %}{% for tool in tools %}{% for name in pi_tool_names[tool] %}{% set _ = names.append(name) %}{% endfor %}{% endfor %}{{ names | join(", ") }}{% endmacro %}"#;

    #[test]
    fn append_in_nested_loops_of_harness_macro() {
        let src = format!("{PI_TOOLS}{{{{ pi_tools(tools) }}}}");
        let out = render(&src, json!({"tools": ["Read", "Glob"]})).unwrap();
        assert_eq!(out, "read, find, ls");
    }

    #[test]
    fn mutable_list_length_count_and_visibility() {
        let out = render(
            "{% set x = [] %}{% for i in [1, 2, 1] %}{% set _ = x.append(i) %}{% endfor %}{{ x | length }}|{{ x.count(1) }}|{{ x | join('-') }}|{{ x }}",
            json!({}),
        )
        .unwrap();
        assert_eq!(out, "3|2|1-2-1|[1, 2, 1]");
    }

    #[test]
    fn append_on_other_list_gives_hint() {
        let err = render("{% set x = [1] %}{% set _ = x.append(2) %}", json!({})).unwrap_err();
        assert!(
            err.to_string()
                .contains("`append` works only on a list created with `{% set name = [] %}`"),
            "{err}"
        );
    }

    #[test]
    fn set_forms_are_rewritten_with_same_output_as_plain_list() {
        let opts = JinjaOptions {
            trim_blocks: true,
            lstrip_blocks: true,
            keep_trailing_newline: true,
        };
        let j = Jinja::new(&opts, Path::new("."));
        let c = Answers::new();
        let src = "  {%- set x = [] -%}\n  {% set y=[] %}\n[{{ x | length }}{{ y | length }}]\n";
        let rewritten = j.render_str("t", src, &c).unwrap();
        let plain_list = j
            .render_str(
                "t",
                "  {%- set x = [1][:0] -%}\n  {% set y=[1][:0] %}\n[{{ x | length }}{{ y | length }}]\n",
                &c,
            )
            .unwrap();
        assert_eq!(rewritten, plain_list);
        let appended = j
            .render_str(
                "t",
                "{%- set x = [] -%}{% set y=[] %}{% set _ = x.append(1) %}{% set _ = y.append(2) %}{{ x }}{{ y }}",
                &c,
            )
            .unwrap();
        assert_eq!(appended, "[1][2]");
    }

    #[test]
    fn set_text_in_raw_comment_and_string_is_untouched() {
        let c = Answers::new();
        let raw = "{% raw %}{% set x = [] %}{% endraw %}";
        assert_eq!(
            plain().render_str("t", raw, &c).unwrap(),
            "{% set x = [] %}"
        );
        let comment = "a{# {% set x = [] %} #}b";
        assert_eq!(plain().render_str("t", comment, &c).unwrap(), "ab");
        let string = r#"{{ "{% set x = [] %}" }}"#;
        assert_eq!(
            plain().render_str("t", string, &c).unwrap(),
            "{% set x = [] %}"
        );
        for s in [raw, comment, string] {
            assert_eq!(rewrite_list_literals(s), s);
        }
    }

    #[test]
    fn rewrite_replaces_only_the_set_form() {
        assert_eq!(
            rewrite_list_literals("{% set x = [] %}"),
            "{% set x = __rootstock_list() %}"
        );
        assert_eq!(
            rewrite_list_literals("{%- set x=[] -%}"),
            "{%- set x=__rootstock_list() -%}"
        );
        assert_eq!(
            rewrite_list_literals("{% set x = [] %}{% raw %}{% set y = [] %}{% endraw %}"),
            "{% set x = __rootstock_list() %}{% raw %}{% set y = [] %}{% endraw %}"
        );
        assert_eq!(
            rewrite_list_literals("{% set x = [1] %}{% set y = [] | list %}"),
            "{% set x = [1] %}{% set y = [] | list %}"
        );
    }

    #[test]
    fn rewrite_leaves_other_text_byte_for_byte() {
        let s = "plain {{ x }} {# c #}\r\n{% if a %}[]{% endif %}\u{e9}{ {% unterminated";
        assert_eq!(rewrite_list_literals(s), s);
    }

    #[test]
    fn append_works_in_included_file_and_in_file_names() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("inc.jinja"),
            "{% set n = [] %}{% set _ = n.append('i') %}{{ n | join(',') }}",
        )
        .unwrap();
        let j = Jinja::new(&JinjaOptions::default(), dir.path());
        let c = Answers::new();
        assert_eq!(
            j.render_str("t", "{% include 'inc.jinja' %}", &c).unwrap(),
            "i"
        );
        let name = "{% set n = [] %}{% set _ = n.append('f') %}{{ n[0] }}.md";
        assert_eq!(j.render_str("name", name, &c).unwrap(), "f.md");
    }

    #[test]
    fn eval_bool_calls_methods() {
        let c = answers(json!({"p": "a.md"}));
        assert!(plain().eval_bool("p.endswith('.md')", &c).unwrap());
    }

    #[test]
    fn rewrite_does_not_panic_on_non_ascii_or_unclosed_tags() {
        for s in [
            "{%\u{a0}if x %}{% set y = [] %}",
            "{% raw %}{% пример %}{% endraw %}",
            "{% if x\nпривет",
            "{{ привет",
            "{# привет",
            "é{é{%é",
        ] {
            let _ = rewrite_list_literals(s);
        }
        let s = "{% if x\nпривет";
        assert_eq!(rewrite_list_literals(s), s);
        let s = "{% raw %}{% пример %}{% endraw %}";
        assert_eq!(rewrite_list_literals(s), s);
        assert_eq!(
            rewrite_list_literals("{%\u{a0}if x %}{% set y = [] %}"),
            "{%\u{a0}if x %}{% set y = __rootstock_list() %}"
        );
        assert_eq!(
            rewrite_list_literals("{% set имя = [] %}"),
            "{% set имя = __rootstock_list() %}"
        );
    }

    #[test]
    fn expression_closes_only_at_bracket_depth_zero() {
        let s = r#"{{ {"a": {"b": 1}} ~ "{% set x = [] %}" }}"#;
        assert_eq!(rewrite_list_literals(s), s);
        let s = r#"{{ {"a": {"b": 1}} }}{% set x = [] %}"#;
        assert_eq!(
            rewrite_list_literals(s),
            r#"{{ {"a": {"b": 1}} }}{% set x = __rootstock_list() %}"#
        );
    }

    #[test]
    fn raw_end_is_matched_lexically() {
        let c = Answers::new();
        for src in [
            "{% raw %}{% it's %}{% endraw %}{% set x = [] %}{% set _ = x.append(1) %}{{ x }}",
            "{% raw %}a {% b{% endraw %}{% set x = [] %}{% set _ = x.append(1) %}{{ x }}",
            "{% raw %}q{%- endraw -%}{% set x = [] %}{% set _ = x.append(1) %}{{ x }}",
        ] {
            let out = plain().render_str("t", src, &c).unwrap();
            assert!(out.ends_with("[1]"), "{out}");
        }
    }

    #[test]
    fn append_argument_count_is_checked() {
        for call in ["x.append()", "x.append(1, 2)"] {
            let src = format!("{{% set x = [] %}}{{% set _ = {call} %}}");
            let err = render(&src, json!({})).unwrap_err().to_string();
            assert!(err.contains("`append` takes exactly one argument"), "{err}");
            assert!(!err.contains("works only on a list"), "{err}");
        }
    }

    #[test]
    fn spaces_inside_brackets_are_rewritten() {
        let src = "{% set x = [ ] %}{% set _ = x.append(1) %}{{ x }}";
        assert_eq!(render(src, json!({})).unwrap(), "[1]");
        let opts = JinjaOptions {
            trim_blocks: true,
            lstrip_blocks: true,
            keep_trailing_newline: true,
        };
        let j = Jinja::new(&opts, Path::new("."));
        let c = Answers::new();
        let a = j.render_str("t", "  {% set x = [ ] %}\nA\n", &c).unwrap();
        let b = j.render_str("t", "  {% set x = [] %}\nA\n", &c).unwrap();
        assert_eq!(a, b);
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
