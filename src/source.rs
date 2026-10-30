//! Template sources: tag listing and checkouts through the `git` CLI.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context, Result, anyhow, bail};
use semver::Version;
use tempfile::TempDir;

use crate::config::CONFIG_FILE;

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

/// Runs `git` and returns its output; stdin is never inherited.
pub(crate) fn run_git(args: &[&str]) -> Result<std::process::Output> {
    Command::new("git")
        // Repository hooks must never run on behalf of a template.
        .args(["-c", "core.hooksPath=/dev/null"])
        // The `ext::` transport runs arbitrary commands.
        .args(["-c", "protocol.ext.allow=never"])
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(Stdio::null())
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
    _dir: Option<TempDir>,
}

impl Checkout {
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
    let real_top = Path::new(&top)
        .canonicalize()
        .with_context(|| format!("cannot resolve `{top}`"))?;
    let real_dir = dir
        .canonicalize()
        .with_context(|| format!("cannot resolve `{dir_str}`"))?;
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
    if prefix.is_empty() {
        return Ok(clone.to_path_buf());
    }
    let not_found = || anyhow!("template not found at `{prefix}` in {rev}");
    let mut dir = clone.to_path_buf();
    for segment in prefix.split('/') {
        dir.push(segment);
        match std::fs::symlink_metadata(&dir) {
            Ok(meta) if meta.file_type().is_dir() => {}
            _ => return Err(not_found()),
        }
    }
    match std::fs::symlink_metadata(dir.join(CONFIG_FILE)) {
        Ok(meta) if meta.file_type().is_file() => Ok(dir),
        _ => Err(not_found()),
    }
}

fn is_full_sha(rev: &str) -> bool {
    rev.len() == 40 && rev.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// Checks out a tag or a full sha of the repository; the checkout's path is
/// the template directory inside it. A local source with `HEAD` is used as-is.
pub fn checkout(src: &Source, rev: &str) -> Result<Checkout> {
    let Located { location, prefix } = locate(src)?;
    if let (Source::Local(dir), "HEAD") = (src, rev) {
        let dir_str = dir.to_string_lossy();
        let commit = git(&["-C", &dir_str, "rev-parse", "HEAD"])
            .with_context(|| format!("{dir_str} has no commits"))?;
        return Ok(Checkout {
            path: dir.clone(),
            commit,
            _dir: None,
        });
    }

    let tmp = TempDir::new().context("failed to create a temporary directory")?;
    let tmp_str = tmp.path().to_string_lossy().into_owned();
    git(&[
        "clone",
        "--quiet",
        "--no-checkout",
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
    Ok(Checkout {
        path: template_dir(tmp.path(), &prefix, rev)?,
        commit,
        _dir: Some(tmp),
    })
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
