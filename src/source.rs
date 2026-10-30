//! Template sources: tag listing and checkouts through the `git` CLI.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context, Result, anyhow, bail};
use semver::Version;
use tempfile::TempDir;

use crate::config::CONFIG_FILE;
use crate::copier::COPIER_FILES;
use crate::render::{FileMode, Modes};

/// Where a template comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    /// A remote URL (or any non-directory string) handed to git verbatim.
    Git(String),
    /// A local directory in a git work tree: its root or a subdirectory.
    Local(PathBuf),
}

impl Source {
    /// The location as git receives it.
    fn location(&self) -> String {
        match self {
            Source::Git(url) => url.clone(),
            Source::Local(path) => path.to_string_lossy().into_owned(),
        }
    }
}

/// Removes the Windows verbatim prefix from a path string: `\\?\C:\x` becomes
/// `C:\x`, `\\?\UNC\server\share` becomes `\\server\share`. Git takes a
/// verbatim path for a URL and refuses it. Anything else is returned as is.
pub fn strip_verbatim(path: &str) -> String {
    if let Some(unc) = path.strip_prefix(r"\\?\UNC\") {
        return format!(r"\\{unc}");
    }
    match path.strip_prefix(r"\\?\") {
        Some(rest) if is_drive_path(rest) => rest.to_string(),
        _ => path.to_string(),
    }
}

/// True for `X:` followed by a separator or nothing.
fn is_drive_path(path: &str) -> bool {
    matches!(path.as_bytes(), [letter, b':'] | [letter, b':', b'\\' | b'/', ..] if letter.is_ascii_alphabetic())
}

/// `Path::canonicalize` for a path that goes to git or into a record: on
/// Windows without the verbatim prefix `canonicalize` adds. Elsewhere the same.
pub fn canonical(path: &Path) -> std::io::Result<PathBuf> {
    let real = path.canonicalize()?;
    if cfg!(windows) {
        Ok(PathBuf::from(strip_verbatim(&real.to_string_lossy())))
    } else {
        Ok(real)
    }
}

/// Classifies a user-supplied source string.
pub fn parse_source(s: &str) -> Result<Source> {
    if let Some(rest) = s.strip_prefix("gh:") {
        return Ok(Source::Git(format!("https://github.com/{rest}.git")));
    }
    let path = Path::new(s);
    if path.is_dir() {
        return Ok(Source::Local(path.to_path_buf()));
    }
    if carries_credentials(s) {
        // The URL is never echoed: it holds a secret.
        bail!("template URL carries credentials; configure a git credential helper instead");
    }
    Ok(Source::Git(s.to_string()))
}

/// A `scheme://` URL whose userinfo holds a password, or an http(s) URL
/// with any userinfo (a token often stands in as the user name).
fn carries_credentials(url: &str) -> bool {
    let Some((scheme, rest)) = url.split_once("://") else {
        return false;
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    let Some((userinfo, _host)) = authority.rsplit_once('@') else {
        return false;
    };
    let web = scheme.eq_ignore_ascii_case("http") || scheme.eq_ignore_ascii_case("https");
    userinfo.contains(':') || (web && !userinfo.is_empty())
}

/// Repository-local variables of `git rev-parse --local-env-vars` (git 2.54).
/// They are removed from every spawned git so an inherited `GIT_DIR` and the
/// like cannot redirect it away from the directory it is meant to work in.
pub(crate) const GIT_LOCAL_ENV: &[&str] = &[
    "GIT_ALTERNATE_OBJECT_DIRECTORIES",
    "GIT_CONFIG",
    "GIT_CONFIG_PARAMETERS",
    "GIT_CONFIG_COUNT",
    "GIT_OBJECT_DIRECTORY",
    "GIT_DIR",
    "GIT_WORK_TREE",
    "GIT_IMPLICIT_WORK_TREE",
    "GIT_GRAFT_FILE",
    "GIT_INDEX_FILE",
    "GIT_NO_REPLACE_OBJECTS",
    "GIT_REPLACE_REF_BASE",
    "GIT_PREFIX",
    "GIT_SHALLOW_FILE",
    "GIT_COMMON_DIR",
];

/// A `program` command (normally `git`) without inherited repository-local
/// variables and with stdin closed.
pub(crate) fn git_command(program: &str) -> Command {
    let mut cmd = Command::new(program);
    for var in GIT_LOCAL_ENV {
        cmd.env_remove(var);
    }
    cmd.stdin(Stdio::null());
    cmd
}

/// Runs `git` and returns its output; stdin is never inherited.
pub(crate) fn run_git(args: &[&str]) -> Result<std::process::Output> {
    git_command("git")
        // Repository hooks must never run on behalf of a template.
        .args(["-c", "core.hooksPath=/dev/null"])
        // The `ext::` transport runs arbitrary commands.
        .args(["-c", "protocol.ext.allow=never"])
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .with_context(|| format!("failed to run git {}", args.join(" ")))
}

/// Runs `git` and returns trimmed stdout, failing with args and stderr.
pub(crate) fn git(args: &[&str]) -> Result<String> {
    let out = run_git(args)?;
    if !out.status.success() {
        bail!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Lists `vX.Y.Z` tags of the source, ascending by version.
pub fn tags(src: &Source) -> Result<Vec<(Version, String)>> {
    let location = locate(src)?.location;
    let out = git(&["ls-remote", "--tags", "--refs", "--", &location])?;
    let mut found: Vec<(Version, String)> = out
        .lines()
        .filter_map(|line| {
            let name = line.split_whitespace().nth(1)?.strip_prefix("refs/tags/")?;
            let version = Version::parse(name.strip_prefix('v')?).ok()?;
            Some((version, name.to_string()))
        })
        .collect();
    found.sort();
    Ok(found)
}

/// The tag of the highest non-pre-release version.
pub fn latest(src: &Source) -> Result<String> {
    tags(src)?
        .into_iter()
        .rev()
        .find(|(v, _)| v.pre.is_empty())
        .map(|(_, tag)| tag)
        .ok_or_else(|| anyhow!("no release tags (vX.Y.Z) found in {}", src.location()))
}

/// A checked-out template version; a temporary directory lives as long as the value.
#[derive(Debug)]
pub struct Checkout {
    /// The template directory: the one holding `rootstock.toml`.
    pub path: PathBuf,
    /// The repository commit.
    pub commit: String,
    /// The version as the user knows it (a tag or `HEAD`); used in messages.
    pub version: String,
    /// Git modes of the template paths, relative to `path`.
    pub modes: Modes,
    _dir: Option<TempDir>,
}

impl Checkout {
    /// Replaces the version named in messages (for a checkout made by commit).
    pub fn named(mut self, version: &str) -> Self {
        self.version = version.to_string();
        self
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

/// What git reads for a source, and where the template lives inside it.
struct Located {
    /// The repository as git receives it.
    location: String,
    /// The template directory relative to the repository root, `/`-separated;
    /// empty when the template is the repository root.
    prefix: String,
}

/// Rejects locations git could read as an option. A local directory may be
/// the root of a git work tree or any directory inside one.
fn locate(src: &Source) -> Result<Located> {
    let location = src.location();
    if location.starts_with('-') {
        bail!("invalid template source `{location}`");
    }
    let Source::Local(dir) = src else {
        return Ok(Located {
            location,
            prefix: String::new(),
        });
    };
    let dir_str = dir.to_string_lossy();
    let out = run_git(&["-C", &dir_str, "rev-parse", "--show-toplevel"])?;
    if !out.status.success() {
        bail!(
            "`{dir_str}` is not a git repository: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    let top = String::from_utf8_lossy(&out.stdout).trim().to_string();
    let real_top = canonical(Path::new(&top)).with_context(|| format!("cannot resolve `{top}`"))?;
    let real_dir = canonical(dir).with_context(|| format!("cannot resolve `{dir_str}`"))?;
    let rel = real_dir
        .strip_prefix(&real_top)
        .map_err(|_| anyhow!("`{dir_str}` is not inside its git repository (root is `{top}`)"))?;
    let mut segments = Vec::new();
    for component in rel.components() {
        let segment = component
            .as_os_str()
            .to_str()
            .ok_or_else(|| anyhow!("`{dir_str}`: path is not valid UTF-8"))?;
        segments.push(segment);
    }
    Ok(Located {
        location: real_top.to_string_lossy().into_owned(),
        prefix: segments.join("/"),
    })
}

/// The template directory `prefix` inside a checked-out clone: every
/// component a real directory (no symlink), holding `rootstock.toml`.
fn template_dir(clone: &Path, prefix: &str, rev: &str) -> Result<PathBuf> {
    template_dir_shown(clone, prefix, prefix, rev)
}

/// Like `template_dir`, walking `walk` from `base`; errors name `shown`,
/// the full path of the template from the repository root.
fn template_dir_shown(base: &Path, walk: &str, shown: &str, rev: &str) -> Result<PathBuf> {
    if walk.is_empty() {
        return Ok(base.to_path_buf());
    }
    let not_found = || anyhow!("template not found at `{shown}` in {rev}");
    let mut dir = base.to_path_buf();
    for segment in walk.split('/') {
        dir.push(segment);
        match std::fs::symlink_metadata(&dir) {
            Ok(meta) if meta.file_type().is_dir() => {}
            _ => return Err(not_found()),
        }
    }
    // A Copier-only directory is found too: the caller decides what to do with it.
    let found = std::iter::once(CONFIG_FILE)
        .chain(COPIER_FILES)
        .any(|name| {
            std::fs::symlink_metadata(dir.join(name)).is_ok_and(|meta| meta.file_type().is_file())
        });
    if found { Ok(dir) } else { Err(not_found()) }
}

fn is_full_sha(rev: &str) -> bool {
    rev.len() == 40 && rev.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// Parses a user-supplied template subdirectory into a `/`-separated path.
/// One trailing `/` is dropped; empty, absolute and escaping paths, and
/// paths with `\`, `:` or NUL, are rejected. Runs before any git access.
pub fn parse_subdir(s: &str) -> Result<String> {
    let path = s.strip_suffix('/').unwrap_or(s);
    let bad = path.is_empty()
        || path.starts_with('/')
        || path.contains(['\\', ':', '\0'])
        || path.split('/').any(|c| matches!(c, "" | "." | ".."));
    if bad {
        bail!("invalid template subdirectory `{}`", s.replace('\0', "\\0"));
    }
    Ok(path.to_string())
}

/// Checks out a tag or a full sha of the repository; the checkout's path is
/// the template directory inside it, `subdir` below the source's own location.
/// A local source with `HEAD` is used as-is (no subdir) or at its subdir.
pub fn checkout(src: &Source, subdir: Option<&str>, rev: &str) -> Result<Checkout> {
    let Located {
        location,
        prefix: offset,
    } = locate(src)?;
    let sub = subdir.unwrap_or("");
    let prefix = match (offset.is_empty(), sub.is_empty()) {
        (_, true) => offset,
        (true, false) => sub.to_string(),
        (false, false) => format!("{offset}/{sub}"),
    };
    if let (Source::Local(dir), "HEAD") = (src, rev) {
        let dir_str = dir.to_string_lossy();
        let commit = git(&["-C", &dir_str, "rev-parse", "HEAD"])
            .with_context(|| format!("{dir_str} has no commits"))?;
        let path = template_dir_shown(dir, sub, &prefix, rev)?;
        let mut modes = git_modes(
            &["-C", &dir_str, "ls-files", "-s", "-z", "--full-name"],
            &prefix,
        )?;
        if cfg!(unix) {
            // The working tree is what is rendered: on Unix a path is a link only
            // if the disk says so; the index supplies the executable bit alone.
            modes.retain(|_, mode| *mode != FileMode::Symlink);
        }
        return Ok(Checkout {
            path,
            commit,
            version: rev.to_string(),
            modes,
            _dir: None,
        });
    }

    let tmp = TempDir::new().context("failed to create a temporary directory")?;
    let tmp_str = tmp.path().to_string_lossy().into_owned();
    git(&[
        "clone",
        "--quiet",
        "--no-checkout",
        // The user's or the system's `autocrlf` must not change the template's bytes.
        "--config",
        "core.autocrlf=false",
        // `text=auto` in the template's attributes would otherwise follow the platform.
        "--config",
        "core.eol=lf",
        "--",
        &location,
        &tmp_str,
    ])?;

    let commit = resolve(&tmp_str, rev)?
        .ok_or_else(|| anyhow!("template commit or tag `{rev}` not found in {location}"))?;
    git(&[
        "-c",
        "advice.detachedHead=false",
        "-C",
        &tmp_str,
        "checkout",
        "--quiet",
        "--detach",
        &commit,
    ])?;
    let modes = git_modes(
        &[
            "-C",
            &tmp_str,
            "ls-tree",
            "-r",
            "-z",
            "--full-tree",
            &commit,
        ],
        &prefix,
    )?;
    Ok(Checkout {
        path: template_dir(tmp.path(), &prefix, rev)?,
        commit,
        version: rev.to_string(),
        modes,
        _dir: Some(tmp),
    })
}

/// Runs a git listing command (`ls-tree -z` or `ls-files -s -z`) and parses its modes.
pub(crate) fn git_modes(args: &[&str], prefix: &str) -> Result<Modes> {
    let out = run_git(args)?;
    if !out.status.success() {
        bail!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(parse_modes(&out.stdout, prefix))
}

/// Parses `git ls-tree -z` or `git ls-files -s -z` output into modes of the
/// paths below `prefix` (`/`-separated, empty for the repository root), keyed
/// relative to it. Submodules and paths that are not UTF-8 are left out.
pub fn parse_modes(listing: &[u8], prefix: &str) -> Modes {
    let mut modes = Modes::new();
    for record in listing.split(|b| *b == 0) {
        // `<mode> <type or sha> ...<TAB><path>`
        let Some(tab) = record.iter().position(|b| *b == b'\t') else {
            continue;
        };
        let (meta, path) = (&record[..tab], &record[tab + 1..]);
        let (Ok(meta), Ok(path)) = (std::str::from_utf8(meta), std::str::from_utf8(path)) else {
            continue;
        };
        let relative = if prefix.is_empty() {
            path
        } else {
            let Some(rest) = path
                .strip_prefix(prefix)
                .and_then(|rest| rest.strip_prefix('/'))
            else {
                continue;
            };
            rest
        };
        let mode = match meta.split(' ').next() {
            Some("100755") => FileMode::Executable,
            Some("120000") => FileMode::Symlink,
            // A submodule or a directory: nothing to emit.
            Some("160000" | "040000") | None => continue,
            Some(_) => FileMode::File,
        };
        modes.insert(relative.to_string(), mode);
    }
    modes
}

/// Resolves a tag name or sha to a commit in the clone; `None` if absent.
fn resolve(dir: &str, rev: &str) -> Result<Option<String>> {
    let mut candidates = Vec::new();
    if !rev.starts_with('-') {
        candidates.push(format!("refs/tags/{rev}^{{commit}}"));
    }
    if is_full_sha(rev) {
        candidates.push(format!("{rev}^{{commit}}"));
    }
    for candidate in candidates {
        let out = run_git(&["-C", dir, "rev-parse", "--verify", "--quiet", &candidate])?;
        if out.status.success() {
            return Ok(Some(
                String::from_utf8_lossy(&out.stdout).trim().to_string(),
            ));
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    const REFUSAL: &str = "template URL carries credentials";

    #[test]
    fn verbatim_prefix_is_stripped() {
        for (given, expected) in [
            (r"\\?\C:\Users\x", r"C:\Users\x"),
            (r"\\?\c:", "c:"),
            (r"\\?\UNC\server\share\dir", r"\\server\share\dir"),
            // Not verbatim, or verbatim forms with no plain spelling: unchanged.
            (r"C:\Users\x", r"C:\Users\x"),
            (r"\\server\share", r"\\server\share"),
            (r"\\?\Volume{1234}\x", r"\\?\Volume{1234}\x"),
            (r"\\?\C", r"\\?\C"),
            ("/tmp/x", "/tmp/x"),
            ("", ""),
        ] {
            assert_eq!(strip_verbatim(given), expected, "{given}");
        }
    }

    #[test]
    fn canonical_path_is_never_verbatim() {
        let dir = tempfile::tempdir().unwrap();
        let real = canonical(dir.path()).unwrap();
        assert!(!real.to_string_lossy().starts_with(r"\\?\"), "{real:?}");
        assert!(real.is_dir());
    }

    #[test]
    fn refuses_credentials_in_url() {
        for url in [
            "https://tok@github.com/o/r.git",
            "https://u:p@h/r.git",
            "HTTP://tok@h/r.git",
            "ssh://u:p@h/r.git",
        ] {
            let err = parse_source(url).unwrap_err().to_string();
            assert!(err.contains(REFUSAL), "{url}: {err}");
            assert!(!err.contains('@'), "{url}: {err}");
        }
    }

    #[test]
    fn accepts_urls_without_credentials() {
        for url in [
            "ssh://git@h/r.git",
            "git@h:o/r.git",
            "https://h/r.git",
            "https://h/a@b.git",
        ] {
            assert_eq!(parse_source(url).unwrap(), Source::Git(url.to_string()));
        }
        assert_eq!(
            parse_source("gh:o/r").unwrap(),
            Source::Git("https://github.com/o/r.git".to_string())
        );
    }
}
