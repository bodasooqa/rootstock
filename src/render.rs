//! Renders a template checkout into an in-memory tree of project files.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use ignore::gitignore::{Gitignore, GitignoreBuilder};

use crate::RelPath;
use crate::config::{CONFIG_FILE, TemplateConfig};
use crate::jinja::{Jinja, JinjaError};
use crate::record::{Answers, FileClass, RECORD_FILE};

/// The content of one rendered path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Entry {
    File { bytes: Vec<u8>, executable: bool },
    Symlink { target: String },
}

/// A rendered path with its class.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rendered {
    pub entry: Entry,
    pub class: FileClass,
}

/// Everything a template emits, by project-relative path.
pub type RenderedTree = BTreeMap<RelPath, Rendered>;

/// Errors from rendering a template; each names the source path.
#[derive(Debug, thiserror::Error)]
pub enum RenderError {
    #[error("{}: {source}", path.display())]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("{}: file name is not valid UTF-8", path.display())]
    NonUtf8Name { path: PathBuf },
    #[error("{}: file is not valid UTF-8 and cannot be rendered", path.display())]
    NonUtf8Content { path: PathBuf },
    #[error("{}: symlink target is not valid UTF-8", path.display())]
    NonUtf8Link { path: PathBuf },
    #[error("{}: {source}", path.display())]
    Jinja { path: PathBuf, source: JinjaError },
    #[error("{}: rendered path {rendered:?} is not allowed: {reason}", path.display())]
    BadPath {
        path: PathBuf,
        rendered: String,
        reason: &'static str,
    },
    #[error("{}: renders to {RECORD_FILE}, which rootstock reserves", path.display())]
    RecordPath { path: PathBuf },
    #[error("{} and {} both render to {rendered}", first.display(), second.display())]
    Collision {
        first: PathBuf,
        second: PathBuf,
        rendered: String,
    },
    #[error("{} and {} cannot both be emitted: {inner} is inside {outer}", first.display(), second.display())]
    Conflict {
        first: PathBuf,
        second: PathBuf,
        outer: String,
        inner: String,
    },
    #[error("{}: symlink target {target:?} {reason}", path.display())]
    SymlinkTarget {
        path: PathBuf,
        target: String,
        reason: &'static str,
    },
    #[error("invalid file rule {pattern:?}: {source}")]
    BadPattern {
        pattern: String,
        source: ignore::Error,
    },
}

/// Renders the template under `repo_root/cfg.root` with `ctx`.
pub fn render_tree(
    repo_root: &Path,
    cfg: &TemplateConfig,
    jinja: &Jinja,
    ctx: &Answers,
) -> Result<RenderedTree, RenderError> {
    let exclude = build_rules(&cfg.files.exclude, jinja, ctx)?;
    let starter = build_rules(&cfg.files.starter, jinja, ctx)?;
    let partials: Vec<String> = cfg
        .partials
        .iter()
        .map(|p| p.trim_matches('/').trim_start_matches("./").to_string())
        .collect();
    let base = repo_root.join(&cfg.root);
    let mut walk = Walk {
        cfg,
        jinja,
        ctx,
        exclude: &exclude,
        starter: &starter,
        partials: &partials,
        sources: BTreeMap::new(),
        tree: BTreeMap::new(),
    };
    walk.dir(&base, &[], &[])?;
    Ok(walk.tree)
}

struct Walk<'a> {
    cfg: &'a TemplateConfig,
    jinja: &'a Jinja,
    ctx: &'a Answers,
    exclude: &'a Gitignore,
    starter: &'a Gitignore,
    partials: &'a [String],
    sources: BTreeMap<RelPath, PathBuf>,
    tree: RenderedTree,
}

impl Walk<'_> {
    /// `src_rel` and `out_rel` are the source and rendered segments of `dir`.
    fn dir(
        &mut self,
        dir: &Path,
        src_rel: &[String],
        out_rel: &[String],
    ) -> Result<(), RenderError> {
        let io = |source| RenderError::Io {
            path: dir.to_path_buf(),
            source,
        };
        let mut names = Vec::new();
        for item in std::fs::read_dir(dir).map_err(io)? {
            names.push(item.map_err(io)?.file_name());
        }
        names.sort();
        for name in names {
            let path = dir.join(&name);
            let Some(name) = name.to_str() else {
                return Err(RenderError::NonUtf8Name { path });
            };
            if name == ".git"
                || (name == CONFIG_FILE && src_rel.is_empty() && root_is_top(&self.cfg.root))
            {
                continue;
            }
            let meta = std::fs::symlink_metadata(&path).map_err(|source| RenderError::Io {
                path: path.clone(),
                source,
            })?;
            let mut src = src_rel.to_vec();
            src.push(name.to_string());
            let src_display = src.join("/");
            if meta.is_dir() {
                if self.partials.contains(&src_display) {
                    continue;
                }
                let Some(segments) = self.name(&path, &src_display, name)? else {
                    continue;
                };
                let mut out = out_rel.to_vec();
                out.extend(segments);
                self.dir(&path, &src, &out)?;
                continue;
            }
            let templated = meta.is_file()
                && !self.cfg.suffix.is_empty()
                && name.ends_with(&self.cfg.suffix)
                && name.len() > self.cfg.suffix.len();
            let source_name = if templated {
                &name[..name.len() - self.cfg.suffix.len()]
            } else {
                name
            };
            let Some(segments) = self.name(&path, &src_display, source_name)? else {
                continue;
            };
            let mut out = out_rel.to_vec();
            out.extend(segments);
            let rendered = out.join("/");
            if out[0].eq_ignore_ascii_case(RECORD_FILE) {
                return Err(RenderError::RecordPath { path });
            }
            if self
                .exclude
                .matched_path_or_any_parents(&rendered, false)
                .is_ignore()
            {
                continue;
            }
            self.check_conflicts(&rendered, &path)?;
            let entry = if meta.file_type().is_symlink() {
                let target = std::fs::read_link(&path).map_err(|source| RenderError::Io {
                    path: path.clone(),
                    source,
                })?;
                let target = target
                    .to_str()
                    .ok_or_else(|| RenderError::NonUtf8Link { path: path.clone() })?
                    .to_string();
                check_link_target(&path, &out[..out.len() - 1], &target)?;
                Entry::Symlink { target }
            } else {
                let bytes = std::fs::read(&path).map_err(|source| RenderError::Io {
                    path: path.clone(),
                    source,
                })?;
                let bytes = if templated {
                    let text = String::from_utf8(bytes)
                        .map_err(|_| RenderError::NonUtf8Content { path: path.clone() })?;
                    self.jinja
                        .render_str(&src_display, &text, self.ctx)
                        .map_err(|source| RenderError::Jinja {
                            path: path.clone(),
                            source,
                        })?
                        .into_bytes()
                } else {
                    bytes
                };
                Entry::File {
                    bytes,
                    executable: is_executable(&meta),
                }
            };
            let class = if self
                .starter
                .matched_path_or_any_parents(&rendered, false)
                .is_ignore()
            {
                FileClass::Starter
            } else {
                FileClass::Managed
            };
            self.sources.insert(rendered.to_lowercase(), path);
            self.tree.insert(rendered, Rendered { entry, class });
        }
        Ok(())
    }

    /// Rejects a path equal to, inside, or containing an already emitted
    /// path, comparing case-insensitively (macOS/Windows filesystems).
    fn check_conflicts(&self, rendered: &str, path: &Path) -> Result<(), RenderError> {
        let lower = rendered.to_lowercase();
        if let Some(first) = self.sources.get(&lower) {
            return Err(RenderError::Collision {
                first: first.clone(),
                second: path.to_path_buf(),
                rendered: rendered.to_string(),
            });
        }
        let conflict = |outer: &str, inner: &str, first: &PathBuf| RenderError::Conflict {
            first: first.clone(),
            second: path.to_path_buf(),
            outer: outer.to_string(),
            inner: inner.to_string(),
        };
        for (i, _) in lower.match_indices('/') {
            if let Some(first) = self.sources.get(&lower[..i]) {
                return Err(conflict(&lower[..i], rendered, first));
            }
        }
        let prefix = format!("{lower}/");
        if let Some((key, first)) = self.sources.range(prefix.clone()..).next()
            && key.starts_with(&prefix)
        {
            return Err(conflict(rendered, key, first));
        }
        Ok(())
    }

    /// Renders one name; `None` when it renders to the empty string.
    fn name(
        &self,
        path: &Path,
        src_display: &str,
        name: &str,
    ) -> Result<Option<Vec<String>>, RenderError> {
        let rendered = self
            .jinja
            .render_str(src_display, name, self.ctx)
            .map_err(|source| RenderError::Jinja {
                path: path.to_path_buf(),
                source,
            })?;
        if rendered.is_empty() {
            return Ok(None);
        }
        let bad = |reason| RenderError::BadPath {
            path: path.to_path_buf(),
            rendered: rendered.clone(),
            reason,
        };
        if rendered.contains('\0') {
            return Err(bad("contains a NUL byte"));
        }
        if rendered.contains('\\') {
            return Err(bad("contains a backslash"));
        }
        let mut segments = Vec::new();
        for seg in rendered.split('/') {
            match seg {
                "" => return Err(bad("has an empty segment or a leading slash")),
                "." | ".." => return Err(bad("has a `.` or `..` segment")),
                _ => {}
            }
            if seg.eq_ignore_ascii_case(".git") {
                return Err(bad("has a `.git` segment"));
            }
            let b = seg.as_bytes();
            if b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':' {
                return Err(bad("has a drive prefix"));
            }
            segments.push(seg.to_string());
        }
        Ok(Some(segments))
    }
}

/// True when `root` names the repository top (`.`, `./`, empty).
fn root_is_top(root: &str) -> bool {
    Path::new(root)
        .components()
        .all(|c| matches!(c, std::path::Component::CurDir))
}

/// Rejects absolute targets and targets that leave the project root when
/// resolved lexically from the link's directory `dir` (rendered segments).
fn check_link_target(path: &Path, dir: &[String], target: &str) -> Result<(), RenderError> {
    let bad = |reason| RenderError::SymlinkTarget {
        path: path.to_path_buf(),
        target: target.to_string(),
        reason,
    };
    let b = target.as_bytes();
    if target.starts_with('/')
        || target.starts_with('\\')
        || (b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':')
    {
        return Err(bad("is absolute"));
    }
    let mut depth = dir.len();
    for seg in target.split(['/', '\\']) {
        match seg {
            "" | "." => {}
            ".." => {
                if depth == 0 {
                    return Err(bad("escapes the project root"));
                }
                depth -= 1;
            }
            _ => depth += 1,
        }
    }
    Ok(())
}

/// Renders every pattern with `ctx` and compiles them into one matcher.
fn build_rules(
    patterns: &[String],
    jinja: &Jinja,
    ctx: &Answers,
) -> Result<Gitignore, RenderError> {
    let mut builder = GitignoreBuilder::new("/");
    for pattern in patterns {
        let rendered = jinja
            .render_str("file rule", pattern, ctx)
            .map_err(|source| RenderError::Jinja {
                path: PathBuf::from(CONFIG_FILE),
                source,
            })?;
        for line in rendered.lines().map(str::trim).filter(|l| !l.is_empty()) {
            builder
                .add_line(None, line)
                .map_err(|source| RenderError::BadPattern {
                    pattern: line.to_string(),
                    source,
                })?;
        }
    }
    builder.build().map_err(|source| RenderError::BadPattern {
        pattern: patterns.join(", "),
        source,
    })
}

#[cfg(unix)]
fn is_executable(meta: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;
    meta.permissions().mode() & 0o111 != 0
}

#[cfg(not(unix))]
fn is_executable(_meta: &std::fs::Metadata) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn cfg(extra: &str) -> TemplateConfig {
        let text = format!("schema = 1\n{extra}");
        crate::config::parse(&text, Path::new(CONFIG_FILE)).unwrap()
    }

    fn put(root: &Path, rel: &str, content: &str) {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, content).unwrap();
    }

    fn render(
        root: &Path,
        cfg: &TemplateConfig,
        ctx: &Answers,
    ) -> Result<RenderedTree, RenderError> {
        let jinja = Jinja::new(&cfg.jinja, &root.join(&cfg.include_root));
        render_tree(root, cfg, &jinja, ctx)
    }

    fn answers(v: serde_json::Value) -> Answers {
        serde_json::from_value(v).unwrap()
    }

    fn file(t: &RenderedTree, p: &str) -> (String, bool) {
        match &t[p].entry {
            Entry::File { bytes, executable } => {
                (String::from_utf8(bytes.clone()).unwrap(), *executable)
            }
            other => panic!("not a file: {other:?}"),
        }
    }

    #[test]
    fn conditional_names() {
        let d = tempfile::tempdir().unwrap();
        put(d.path(), "{% if rust %}Cargo.toml{% endif %}", "x");
        put(d.path(), "keep", "y");
        let c = cfg("");
        let on = render(d.path(), &c, &answers(json!({"rust": true}))).unwrap();
        assert!(on.contains_key("Cargo.toml") && on.contains_key("keep"));
        let off = render(d.path(), &c, &answers(json!({"rust": false}))).unwrap();
        assert_eq!(off.keys().collect::<Vec<_>>(), ["keep"]);
        assert!(!off.contains_key("rootstock.toml"));
    }

    #[test]
    fn suffix_rendered_others_verbatim() {
        let d = tempfile::tempdir().unwrap();
        put(d.path(), "a.txt.jinja", "hi {{ name }}\n");
        put(d.path(), "b.txt", "raw {{ name }}\n");
        std::fs::write(d.path().join("bin.dat"), [0xff, 0xfe, 0x00]).unwrap();
        let t = render(d.path(), &cfg(""), &answers(json!({"name": "bob"}))).unwrap();
        assert_eq!(file(&t, "a.txt").0, "hi bob\n");
        assert_eq!(file(&t, "b.txt").0, "raw {{ name }}\n");
        assert_eq!(
            t["bin.dat"].entry,
            Entry::File {
                bytes: vec![0xff, 0xfe, 0],
                executable: false
            }
        );
        assert_eq!(t["a.txt"].class, FileClass::Managed);
    }

    #[test]
    fn non_utf8_template_names_file() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("bad.jinja"), [0xff]).unwrap();
        let e = render(d.path(), &cfg(""), &Answers::new()).unwrap_err();
        assert!(e.to_string().contains("bad.jinja"), "{e}");
    }

    #[cfg(unix)]
    #[test]
    fn executable_bit_kept() {
        use std::os::unix::fs::PermissionsExt;
        let d = tempfile::tempdir().unwrap();
        put(d.path(), "run.sh.jinja", "#!/bin/sh\n");
        put(d.path(), "copy.sh", "#!/bin/sh\n");
        put(d.path(), "plain", "x");
        for f in ["run.sh.jinja", "copy.sh"] {
            std::fs::set_permissions(d.path().join(f), std::fs::Permissions::from_mode(0o755))
                .unwrap();
        }
        let t = render(d.path(), &cfg(""), &Answers::new()).unwrap();
        assert!(file(&t, "run.sh").1);
        assert!(file(&t, "copy.sh").1);
        assert!(!file(&t, "plain").1);
    }

    #[cfg(unix)]
    #[test]
    fn symlink_kept() {
        let d = tempfile::tempdir().unwrap();
        put(d.path(), "real.txt", "x");
        std::os::unix::fs::symlink("real.txt", d.path().join("link")).unwrap();
        std::os::unix::fs::symlink("{{ nope }}/dir", d.path().join("dirlink")).unwrap();
        let t = render(d.path(), &cfg(""), &Answers::new()).unwrap();
        assert_eq!(
            t["link"].entry,
            Entry::Symlink {
                target: "real.txt".into()
            }
        );
        assert_eq!(
            t["dirlink"].entry,
            Entry::Symlink {
                target: "{{ nope }}/dir".into()
            }
        );
    }

    #[test]
    fn partials_not_emitted() {
        let d = tempfile::tempdir().unwrap();
        put(d.path(), "_parts/p.txt", "{{ x }}");
        put(d.path(), "a/_parts/q.txt", "q");
        put(d.path(), "main.txt.jinja", "{% include '_parts/p.txt' %}");
        let t = render(
            d.path(),
            &cfg("partials = [\"_parts\"]\n"),
            &answers(json!({"x": "v"})),
        )
        .unwrap();
        assert_eq!(t.len(), 2, "{:?}", t.keys());
        assert_eq!(file(&t, "a/_parts/q.txt").0, "q");
        assert_eq!(file(&t, "main.txt").0, "v");
    }

    #[test]
    fn starter_rule_with_answer() {
        let d = tempfile::tempdir().unwrap();
        put(d.path(), "{{ spec_path }}/a.md", "a");
        put(d.path(), "other.md", "o");
        let c = cfg("[files]\nstarter = [\"{% if spec_path %}{{ spec_path }}/**{% endif %}\"]\n");
        let t = render(d.path(), &c, &answers(json!({"spec_path": "docs/spec"}))).unwrap();
        assert_eq!(t["docs/spec/a.md"].class, FileClass::Starter);
        assert_eq!(t["other.md"].class, FileClass::Managed);
    }

    #[test]
    fn starter_rule_empty_answer_ignored() {
        let d = tempfile::tempdir().unwrap();
        put(d.path(), "docs/spec/a.md", "a");
        let c = cfg("[files]\nstarter = [\"{% if spec_path %}{{ spec_path }}/**{% endif %}\"]\n");
        let t = render(d.path(), &c, &answers(json!({"spec_path": ""}))).unwrap();
        assert_eq!(t["docs/spec/a.md"].class, FileClass::Managed);
    }

    #[test]
    fn exclude_and_root_anchored_rule() {
        let d = tempfile::tempdir().unwrap();
        put(d.path(), "README.md", "r");
        put(d.path(), ".harness/hooks/README.md", "h");
        put(d.path(), "skip/x", "x");
        let c = cfg("[files]\nstarter = [\"/README.md\"]\nexclude = [\"skip\"]\n");
        let t = render(d.path(), &c, &Answers::new()).unwrap();
        assert_eq!(t["README.md"].class, FileClass::Starter);
        assert_eq!(t[".harness/hooks/README.md"].class, FileClass::Managed);
        assert!(!t.contains_key("skip/x"));
    }

    #[test]
    fn subdir_root_and_empty_component_dropped() {
        let d = tempfile::tempdir().unwrap();
        put(d.path(), "tpl/{{ empty }}/gone/x", "x");
        put(d.path(), "tpl/ok", "ok");
        put(d.path(), "outside", "o");
        let a = answers(json!({"empty": ""}));
        let t = render(d.path(), &cfg("root = \"tpl\"\n"), &a).unwrap();
        assert_eq!(t.keys().collect::<Vec<_>>(), ["ok"]);
    }

    #[test]
    fn rendered_name_may_contain_slash() {
        let d = tempfile::tempdir().unwrap();
        put(d.path(), "{{ p }}/a.md", "a");
        let t = render(d.path(), &cfg(""), &answers(json!({"p": "docs/spec"}))).unwrap();
        assert!(t.contains_key("docs/spec/a.md"));
    }

    #[test]
    fn git_dir_skipped() {
        let d = tempfile::tempdir().unwrap();
        put(d.path(), ".git/config", "c");
        put(d.path(), "sub/.git/x", "c");
        put(d.path(), "a", "a");
        let t = render(d.path(), &cfg(""), &Answers::new()).unwrap();
        assert_eq!(t.keys().collect::<Vec<_>>(), ["a"]);
    }

    #[test]
    fn rejects_record_path() {
        let d = tempfile::tempdir().unwrap();
        put(d.path(), "{{ n }}", "x");
        let e = render(
            d.path(),
            &cfg(""),
            &answers(json!({"n": ".rootstock.toml"})),
        )
        .unwrap_err();
        assert!(matches!(e, RenderError::RecordPath { .. }), "{e}");
        let d = tempfile::tempdir().unwrap();
        put(d.path(), ".rootstock.toml.jinja", "x");
        assert!(render(d.path(), &cfg(""), &Answers::new()).is_err());
    }

    #[test]
    fn rejects_escaping_paths() {
        for bad in [
            "..", "/etc/x", "a//b", "a\\b", "C:x", "a/../b", "x\0y", "a/",
        ] {
            let d = tempfile::tempdir().unwrap();
            put(d.path(), "{{ n }}", "x");
            let e = render(d.path(), &cfg(""), &answers(json!({"n": bad}))).unwrap_err();
            assert!(matches!(e, RenderError::BadPath { .. }), "{bad:?}: {e}");
        }
        let d = tempfile::tempdir().unwrap();
        put(d.path(), "{{ n }}/f", "x");
        let r = render(d.path(), &cfg(""), &answers(json!({"n": ".."})));
        assert!(matches!(r, Err(RenderError::BadPath { .. })));
    }

    #[test]
    fn rejects_record_dir_and_git_segments() {
        for bad in [
            ".rootstock.toml/x",
            ".ROOTSTOCK.toml",
            ".git/hooks/pre-commit",
            ".GIT/x",
            "a/.Git",
        ] {
            let d = tempfile::tempdir().unwrap();
            put(d.path(), "{{ n }}", "x");
            let r = render(d.path(), &cfg(""), &answers(json!({"n": bad})));
            assert!(
                matches!(
                    r,
                    Err(RenderError::BadPath { .. } | RenderError::RecordPath { .. })
                ),
                "{bad}: {r:?}"
            );
        }
    }

    #[test]
    fn rejects_case_insensitive_collision() {
        let d = tempfile::tempdir().unwrap();
        put(d.path(), "README.md", "1");
        put(d.path(), "{{ n }}", "2");
        let r = render(d.path(), &cfg(""), &answers(json!({"n": "readme.md"})));
        assert!(matches!(r, Err(RenderError::Collision { .. })), "{r:?}");
    }

    #[test]
    fn rejects_file_dir_conflict() {
        // `a` sorts before `a.d`... use names where either order occurs.
        for (file, nested) in [("a", "{{ n }}/b"), ("{{ n }}", "a/b")] {
            let d = tempfile::tempdir().unwrap();
            put(d.path(), file, "1");
            put(d.path(), nested, "2");
            let r = render(d.path(), &cfg(""), &answers(json!({"n": "a"})));
            assert!(
                matches!(r, Err(RenderError::Conflict { .. })),
                "{file}: {r:?}"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symlink_dir_conflict() {
        let d = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink("x", d.path().join("a")).unwrap();
        put(d.path(), "{{ n }}/f", "1");
        let r = render(d.path(), &cfg(""), &answers(json!({"n": "a"})));
        assert!(matches!(r, Err(RenderError::Conflict { .. })), "{r:?}");
        let d = tempfile::tempdir().unwrap();
        put(d.path(), "a/f", "1");
        std::os::unix::fs::symlink("x", d.path().join("{{ n }}")).unwrap();
        let r = render(d.path(), &cfg(""), &answers(json!({"n": "a"})));
        assert!(matches!(r, Err(RenderError::Conflict { .. })), "{r:?}");
    }

    #[cfg(unix)]
    #[test]
    fn symlink_targets_checked() {
        let link = |target: &str, at: &str| {
            let d = tempfile::tempdir().unwrap();
            let p = d.path().join(at);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::os::unix::fs::symlink(target, p).unwrap();
            render(d.path(), &cfg(""), &Answers::new())
        };
        for (t, at) in [
            ("/etc", "l"),
            ("../x", "l"),
            ("../../x", "a/l"),
            ("a/../../x", "l"),
        ] {
            let r = link(t, at);
            assert!(
                matches!(r, Err(RenderError::SymlinkTarget { .. })),
                "{t}: {r:?}"
            );
        }
        assert!(link("../shared/x", "a/l").is_ok());
        assert!(link("sub/../x", "l").is_ok());
    }

    #[test]
    fn rejects_collisions() {
        let d = tempfile::tempdir().unwrap();
        put(d.path(), "a", "1");
        put(d.path(), "{{ n }}", "2");
        let e = render(d.path(), &cfg(""), &answers(json!({"n": "a"}))).unwrap_err();
        assert!(matches!(e, RenderError::Collision { .. }), "{e}");
    }

    #[test]
    fn jinja_error_names_file() {
        let d = tempfile::tempdir().unwrap();
        put(d.path(), "x.jinja", "{% if %}");
        let e = render(d.path(), &cfg(""), &Answers::new()).unwrap_err();
        assert!(e.to_string().contains("x.jinja"), "{e}");
    }
}
