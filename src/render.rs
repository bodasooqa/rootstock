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
    #[error("{}: template root resolves outside the repository", path.display())]
    RootOutside { path: PathBuf },
    #[error("invalid file rule {pattern:?}: {source}")]
    BadPattern {
        pattern: String,
        source: ignore::Error,
    },
}

/// Canonical include root; it must stay inside the canonical repository.
pub fn include_root(repo_root: &Path, cfg: &TemplateConfig) -> Result<PathBuf, RenderError> {
    let joined = repo_root.join(&cfg.include_root);
    let io = |path: &Path| {
        let path = path.to_path_buf();
        move |source| RenderError::Io { path, source }
    };
    let real_repo = std::fs::canonicalize(repo_root).map_err(io(repo_root))?;
    let real = std::fs::canonicalize(&joined).map_err(io(&joined))?;
    if !real.starts_with(&real_repo) {
        return Err(RenderError::RootOutside { path: joined });
    }
    Ok(real)
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
    let io = |path: &Path| {
        let path = path.to_path_buf();
        move |source| RenderError::Io { path, source }
    };
    let real_repo = std::fs::canonicalize(repo_root).map_err(io(repo_root))?;
    let real_base = std::fs::canonicalize(&base).map_err(io(&base))?;
    if !real_base.starts_with(&real_repo) {
        return Err(RenderError::RootOutside { path: base });
    }
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
    walk.check_symlinks()?;
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
            let top = fold_segment(&out[0]);
            if top == RECORD_FILE || is_short_record(&top) {
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
                check_absolute(&path, &target)?;
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
            self.sources.insert(fold_path(&rendered), path);
            self.tree.insert(rendered, Rendered { entry, class });
        }
        Ok(())
    }

    /// Resolves every symlink target through the emitted tree (following
    /// other emitted links) and rejects any that leaves the project root.
    /// Runs twice: links matched by exact path, then by folded path.
    fn check_symlinks(&self) -> Result<(), RenderError> {
        let link_map = |key: fn(&str) -> String| -> BTreeMap<String, &str> {
            self.tree
                .iter()
                .filter_map(|(p, r)| match &r.entry {
                    Entry::Symlink { target } => Some((key(p), target.as_str())),
                    Entry::File { .. } => None,
                })
                .collect()
        };
        let exact = link_map(str::to_string);
        let folded = link_map(fold_path);
        for (rendered, item) in &self.tree {
            let Entry::Symlink { target } = &item.entry else {
                continue;
            };
            let source = self
                .sources
                .get(&fold_path(rendered))
                .cloned()
                .unwrap_or_else(|| PathBuf::from(rendered));
            let bad = |reason| RenderError::SymlinkTarget {
                path: source.clone(),
                target: target.clone(),
                reason,
            };
            // Filesystems that normalize Unicode (APFS, HFS+) alias an NFD path with an
            // NFC one, which `fold_path` does not model. With link paths and link targets
            // both ASCII, every alias is one `fold_segment` models.
            if !rendered.is_ascii() {
                return Err(bad(
                    "belongs to a link whose own path has a non-ASCII character",
                ));
            }
            // Filesystems fold Unicode (HFS+ NFD) and 8.3 short names in ways we do not
            // model, so targets are restricted to a class where exact and folded
            // resolution are the whole story.
            if !target.is_ascii() {
                return Err(bad("has a non-ASCII character"));
            }
            if target.contains('~') {
                return Err(bad("has a `~`"));
            }
            for seg in target.split(['/', '\\']) {
                if !matches!(seg, "" | "." | "..") {
                    check_segment(seg).map_err(bad)?;
                }
            }
            resolve(rendered, target, &exact, str::to_string).map_err(bad)?;
            resolve(rendered, target, &folded, fold_segment).map_err(bad)?;
        }
        Ok(())
    }

    /// Rejects a path equal to, inside, or containing an already emitted
    /// path, comparing case-insensitively (macOS/Windows filesystems).
    fn check_conflicts(&self, rendered: &str, path: &Path) -> Result<(), RenderError> {
        let lower = fold_path(rendered);
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
            if seg.is_empty() {
                return Err(bad("has an empty segment or a leading slash"));
            }
            check_segment(seg).map_err(bad)?;
            let folded = fold_segment(seg);
            if folded == ".git" || is_short_git(&folded) {
                return Err(bad("has a `.git` segment"));
            }
            segments.push(seg.to_string());
        }
        Ok(Some(segments))
    }
}

/// Rejects a segment with a `:` or one that folds to nothing (`.`, `..`, `...`).
fn check_segment(seg: &str) -> Result<(), &'static str> {
    if seg.contains(':') {
        return Err("has a `:` (drive prefix or alternate data stream)");
    }
    if fold_segment(seg).is_empty() {
        return Err("has a `.` or `..` segment");
    }
    Ok(())
}

/// Follows `target` from the directory of link `rendered`, expanding emitted
/// links found in `links` (keyed by `key` of each path segment).
fn resolve(
    rendered: &str,
    target: &str,
    links: &BTreeMap<String, &str>,
    key: fn(&str) -> String,
) -> Result<(), &'static str> {
    let mut cur: Vec<String> = rendered.split('/').map(String::from).collect();
    cur.pop();
    let mut pending: std::collections::VecDeque<String> =
        target.split(['/', '\\']).map(String::from).collect();
    let mut hops = 0;
    while let Some(seg) = pending.pop_front() {
        match seg.as_str() {
            "" | "." => continue,
            ".." => {
                if cur.pop().is_none() {
                    return Err("escapes the project root");
                }
                continue;
            }
            _ => cur.push(seg),
        }
        let path: Vec<String> = cur.iter().map(|s| key(s)).collect();
        if let Some(next) = links.get(&path.join("/")) {
            hops += 1;
            if hops > MAX_LINK_HOPS {
                return Err("goes through too many symlinks");
            }
            if next.starts_with(['/', '\\']) {
                return Err("goes through an absolute symlink");
            }
            cur.pop();
            for s in next.split(['/', '\\']).rev() {
                pending.push_front(s.to_string());
            }
        }
    }
    Ok(())
}

/// NTFS 8.3 short name of the record file: `rootst~<digits>.tom`.
fn is_short_record(folded: &str) -> bool {
    folded
        .strip_prefix("rootst~")
        .and_then(|r| r.strip_suffix(".tom"))
        .is_some_and(|d| !d.is_empty() && d.bytes().all(|b| b.is_ascii_digit()))
}

/// Folds a path segment the way case-insensitive filesystems alias it:
/// drops HFS+ ignorable code points, trailing `.` and spaces, and case.
fn fold_segment(seg: &str) -> String {
    let kept: String = seg
        .chars()
        .filter(|c| {
            !matches!(*c, '\u{200c}'..='\u{200f}' | '\u{202a}'..='\u{202e}'
                | '\u{206a}'..='\u{206f}' | '\u{feff}')
        })
        .collect();
    kept.trim_end_matches(['.', ' ']).to_lowercase()
}

fn fold_path(path: &str) -> String {
    path.split('/')
        .map(fold_segment)
        .collect::<Vec<_>>()
        .join("/")
}

/// NTFS 8.3 short name of `.git`: `git~<digits>`.
fn is_short_git(folded: &str) -> bool {
    folded
        .strip_prefix("git~")
        .is_some_and(|d| !d.is_empty() && d.bytes().all(|b| b.is_ascii_digit()))
}

/// True when `root` names the repository top (`.`, `./`, empty).
fn root_is_top(root: &str) -> bool {
    Path::new(root)
        .components()
        .all(|c| matches!(c, std::path::Component::CurDir))
}

/// Rejects absolute symlink targets.
fn check_absolute(path: &Path, target: &str) -> Result<(), RenderError> {
    let b = target.as_bytes();
    if target.starts_with(['/', '\\'])
        || (b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':')
    {
        return Err(RenderError::SymlinkTarget {
            path: path.to_path_buf(),
            target: target.to_string(),
            reason: "is absolute",
        });
    }
    Ok(())
}

/// Longest symlink chain followed before giving up.
const MAX_LINK_HOPS: usize = 40;

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
        let jinja = Jinja::new(&cfg.jinja, &include_root(root, cfg)?);
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
    fn rejects_filesystem_aliases() {
        for bad in [
            ".git.",
            ".git ",
            "git~1",
            "GIT~12",
            ".git::$DATA",
            ".g\u{200c}it",
            ".rootstock.toml.",
            ".rootstock.toml ",
            ".rootstock\u{feff}.toml",
            "a/.git...",
        ] {
            let d = tempfile::tempdir().unwrap();
            put(d.path(), "{{ n }}", "x");
            let r = render(d.path(), &cfg(""), &answers(json!({"n": bad})));
            assert!(
                matches!(
                    r,
                    Err(RenderError::BadPath { .. } | RenderError::RecordPath { .. })
                ),
                "{bad:?}: {r:?}"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn symlink_chains_checked() {
        use std::os::unix::fs::symlink;
        let run = |links: &[(&str, &str)]| {
            let d = tempfile::tempdir().unwrap();
            for (at, target) in links {
                let p = d.path().join(at);
                std::fs::create_dir_all(p.parent().unwrap()).unwrap();
                symlink(target, p).unwrap();
            }
            render(d.path(), &cfg(""), &Answers::new())
        };
        let r = run(&[("s", "."), ("l", "s/../x")]);
        assert!(matches!(r, Err(RenderError::SymlinkTarget { .. })), "{r:?}");
        let r = run(&[("d/s", ".."), ("l", "d/s/../..")]);
        assert!(matches!(r, Err(RenderError::SymlinkTarget { .. })), "{r:?}");
        let r = run(&[("a", "b"), ("b", "a"), ("l", "a/x")]);
        assert!(matches!(r, Err(RenderError::SymlinkTarget { .. })), "{r:?}");
        assert!(run(&[("s", "sub"), ("l", "s/file")]).is_ok());
    }

    #[cfg(unix)]
    #[test]
    fn root_symlink_outside_repo_rejected() {
        let d = tempfile::tempdir().unwrap();
        let repo = d.path().join("repo");
        let outside = d.path().join("outside");
        std::fs::create_dir_all(&repo).unwrap();
        put(&outside, "secret", "s");
        std::os::unix::fs::symlink(&outside, repo.join("tpl")).unwrap();
        let r = render(&repo, &cfg("root = \"tpl\"\n"), &Answers::new());
        assert!(matches!(r, Err(RenderError::RootOutside { .. })), "{r:?}");
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

    #[cfg(unix)]
    #[test]
    fn include_root_symlink_outside_rejected() {
        let d = tempfile::tempdir().unwrap();
        let repo = d.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        put(&d.path().join("outside"), "x", "s");
        std::os::unix::fs::symlink(d.path().join("outside"), repo.join("inc")).unwrap();
        let r = include_root(&repo, &cfg("include_root = \"inc\"\n"));
        assert!(matches!(r, Err(RenderError::RootOutside { .. })), "{r:?}");
    }

    #[test]
    fn include_root_inside_ok() {
        let d = tempfile::tempdir().unwrap();
        put(d.path(), "inc/x", "s");
        let got = include_root(d.path(), &cfg("include_root = \"inc\"\n")).unwrap();
        assert_eq!(got, std::fs::canonicalize(d.path().join("inc")).unwrap());
    }

    #[test]
    fn rejects_short_record_alias() {
        for bad in ["ROOTST~1.TOM", "rootst~12.tom"] {
            let d = tempfile::tempdir().unwrap();
            put(d.path(), "{{ n }}", "x");
            let r = render(d.path(), &cfg(""), &answers(json!({"n": bad})));
            assert!(
                matches!(r, Err(RenderError::RecordPath { .. })),
                "{bad}: {r:?}"
            );
        }
        let d = tempfile::tempdir().unwrap();
        put(d.path(), "sub/rootst~1.tom", "x");
        assert!(render(d.path(), &cfg(""), &Answers::new()).is_ok());
    }

    #[cfg(unix)]
    #[test]
    fn symlink_target_names_checked() {
        for t in [".../x", ".. /x", "a:b", "x/a:b", ". /x"] {
            let d = tempfile::tempdir().unwrap();
            std::os::unix::fs::symlink(t, d.path().join("l")).unwrap();
            let r = render(d.path(), &cfg(""), &Answers::new());
            assert!(
                matches!(r, Err(RenderError::SymlinkTarget { .. })),
                "{t}: {r:?}"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn symlink_exact_and_folded_resolution() {
        use std::os::unix::fs::symlink;
        let d = tempfile::tempdir().unwrap();
        symlink("sub/deep", d.path().join("s")).unwrap();
        symlink("S/../../..", d.path().join("l")).unwrap();
        let r = render(d.path(), &cfg(""), &Answers::new());
        assert!(matches!(r, Err(RenderError::SymlinkTarget { .. })), "{r:?}");
    }

    #[cfg(unix)]
    #[test]
    fn symlink_target_non_ascii_rejected() {
        for t in ["caf\u{e9}/x", "a\u{301}"] {
            let d = tempfile::tempdir().unwrap();
            std::os::unix::fs::symlink(t, d.path().join("l")).unwrap();
            let r = render(d.path(), &cfg(""), &Answers::new());
            assert!(
                matches!(r, Err(RenderError::SymlinkTarget { .. })),
                "{t}: {r:?}"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn symlink_path_non_ascii_rejected() {
        for l in ["caf\u{e9}/l", "\u{e9}"] {
            let d = tempfile::tempdir().unwrap();
            if let Some((dir, _)) = l.split_once('/') {
                std::fs::create_dir(d.path().join(dir)).unwrap();
            }
            std::os::unix::fs::symlink("x", d.path().join(l)).unwrap();
            let r = render(d.path(), &cfg(""), &Answers::new());
            assert!(
                matches!(r, Err(RenderError::SymlinkTarget { .. })),
                "{l}: {r:?}"
            );
        }
    }

    #[test]
    fn non_ascii_regular_file_allowed() {
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir(d.path().join("caf\u{e9}")).unwrap();
        std::fs::write(d.path().join("caf\u{e9}/f"), "x").unwrap();
        let r = render(d.path(), &cfg(""), &Answers::new());
        assert!(r.is_ok(), "{r:?}");
    }

    #[cfg(unix)]
    #[test]
    fn symlink_target_tilde_rejected() {
        for t in ["ROOTST~1/x", "a~b"] {
            let d = tempfile::tempdir().unwrap();
            std::os::unix::fs::symlink(t, d.path().join("l")).unwrap();
            let r = render(d.path(), &cfg(""), &Answers::new());
            assert!(
                matches!(r, Err(RenderError::SymlinkTarget { .. })),
                "{t}: {r:?}"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn symlink_exact_only_escape_rejected() {
        use std::os::unix::fs::symlink;
        let d = tempfile::tempdir().unwrap();
        put(d.path(), "a/sub/deep/f", "x");
        symlink("sub/deep", d.path().join("a/s")).unwrap();
        symlink("S/../../../x", d.path().join("a/l")).unwrap();
        let r = render(d.path(), &cfg(""), &Answers::new());
        assert!(matches!(r, Err(RenderError::SymlinkTarget { .. })), "{r:?}");
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
