//! Template sources: tag listing and checkouts through the `git` CLI.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context, Result, anyhow, bail};
use semver::Version;
use tempfile::TempDir;

/// Where a template comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    /// A remote URL (or any non-directory string) handed to git verbatim.
    Git(String),
    /// A local directory holding a git repository.
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
pub fn parse_source(s: &str) -> Source {
    if let Some(rest) = s.strip_prefix("gh:") {
        return Source::Git(format!("https://github.com/{rest}.git"));
    }
    let path = Path::new(s);
    if path.is_dir() {
        return Source::Local(path.to_path_buf());
    }
    Source::Git(s.to_string())
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
    let location = checked_location(src)?;
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
    pub path: PathBuf,
    pub commit: String,
    _dir: Option<TempDir>,
}

impl Checkout {
    pub fn path(&self) -> &Path {
        &self.path
    }
}

/// Rejects locations git could read as an option, and local directories
/// that are not the root of a git work tree.
fn checked_location(src: &Source) -> Result<String> {
    let location = src.location();
    if location.starts_with('-') {
        bail!("invalid template source `{location}`");
    }
    if let Source::Local(dir) = src {
        let dir_str = dir.to_string_lossy();
        let out = run_git(&["-C", &dir_str, "rev-parse", "--show-toplevel"])?;
        if !out.status.success() {
            bail!(
                "`{dir_str}` is not a git repository: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            );
        }
        let top = String::from_utf8_lossy(&out.stdout).trim().to_string();
        let same = match (Path::new(&top).canonicalize(), dir.canonicalize()) {
            (Ok(a), Ok(b)) => a == b,
            _ => false,
        };
        if !same {
            bail!("`{dir_str}` is not the root of a git repository (root is `{top}`)");
        }
    }
    Ok(location)
}

fn is_full_sha(rev: &str) -> bool {
    rev.len() == 40 && rev.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// Checks out a tag or a full sha. A local source with `HEAD` is used as-is.
pub fn checkout(src: &Source, rev: &str) -> Result<Checkout> {
    let location = checked_location(src)?;
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
        path: tmp.path().to_path_buf(),
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
