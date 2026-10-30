//! Test fixtures shared by the integration tests.
#![allow(dead_code)]

use std::fs;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::Command;

use tempfile::TempDir;

/// A git repository in a temporary directory that plays the role of a template.
pub struct TemplateRepo {
    pub dir: TempDir,
}

impl TemplateRepo {
    pub fn new() -> Self {
        let repo = TemplateRepo {
            dir: TempDir::new().unwrap(),
        };
        repo.git(&["init", "-q", "-b", "main"]);
        repo.git(&["config", "user.name", "Test"]);
        repo.git(&["config", "user.email", "test@example.com"]);
        repo.git(&["config", "commit.gpgsign", "false"]);
        repo.git(&["config", "tag.gpgsign", "false"]);
        repo
    }

    pub fn git(&self, args: &[&str]) -> String {
        let out = Command::new("git")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .arg("-C")
            .arg(self.dir.path())
            .args(args)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    pub fn file(&self, path: &str, content: &str) -> &Self {
        let full = self.dir.path().join(path);
        if let Some(parent) = full.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(&full, content).unwrap();
        self
    }

    /// Writes raw bytes, creating parent directories.
    pub fn bytes(&self, path: &str, content: &[u8]) -> &Self {
        let full = self.dir.path().join(path);
        if let Some(parent) = full.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(&full, content).unwrap();
        self
    }

    /// Makes the file executable: in the git index on every OS (the only
    /// place Windows keeps the bit), and on the disk where it has one.
    pub fn exec(&self, path: &str) -> &Self {
        #[cfg(unix)]
        {
            let full = self.dir.path().join(path);
            fs::set_permissions(&full, fs::Permissions::from_mode(0o755)).unwrap();
        }
        self.git(&["add", "--", path]);
        self.git(&["update-index", "--chmod=+x", "--", path]);
        self
    }

    /// Takes the executable bit off the file, in the index and where the disk has one.
    pub fn unexec(&self, path: &str) -> &Self {
        #[cfg(unix)]
        {
            let full = self.dir.path().join(path);
            fs::set_permissions(&full, fs::Permissions::from_mode(0o644)).unwrap();
        }
        self.git(&["add", "--", path]);
        self.git(&["update-index", "--chmod=-x", "--", path]);
        self
    }

    /// Records a symlink in the git tree without touching the disk: the way
    /// to build a `120000` entry on any OS, whatever `core.symlinks` says.
    pub fn tree_symlink(&self, path: &str, target: &str) -> &Self {
        use std::io::Write;
        use std::process::Stdio;
        let mut child = Command::new("git")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .arg("-C")
            .arg(self.dir.path())
            .args(["hash-object", "-w", "--stdin"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(target.as_bytes())
            .unwrap();
        let out = child.wait_with_output().unwrap();
        assert!(out.status.success());
        let sha = String::from_utf8_lossy(&out.stdout).trim().to_string();
        self.git(&[
            "update-index",
            "--add",
            "--cacheinfo",
            &format!("120000,{sha},{path}"),
        ]);
        self
    }

    /// Creates (or replaces) a symlink at `path` pointing to `target`.
    pub fn symlink(&self, path: &str, target: &str) -> &Self {
        let full = self.dir.path().join(path);
        if let Some(parent) = full.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        let _ = fs::remove_file(&full);
        make_symlink(target, &full);
        self
    }

    pub fn remove(&self, path: &str) -> &Self {
        fs::remove_file(self.dir.path().join(path)).unwrap();
        self
    }

    /// Commits everything and tags the commit.
    pub fn tag(&self, tag: &str) -> &Self {
        self.git(&["add", "-A"]);
        self.git(&["commit", "--allow-empty", "-q", "-m", tag]);
        self.git(&["tag", tag]);
        self
    }

    /// Like `tag`, but stages new and changed files without removals: `add -A`
    /// would drop a link that `tree_symlink` put only into the index.
    pub fn tag_staged(&self, tag: &str) -> &Self {
        self.git(&["add", "--ignore-removal", "."]);
        self.git(&["commit", "--allow-empty", "-q", "-m", tag]);
        self.git(&["tag", tag]);
        self
    }

    /// Commits everything and creates an annotated tag.
    pub fn tag_annotated(&self, tag: &str) -> &Self {
        self.git(&["add", "-A"]);
        self.git(&["commit", "--allow-empty", "-q", "-m", tag]);
        self.git(&["tag", "-a", "-m", tag, tag]);
        self
    }

    /// Creates a branch at the current commit.
    pub fn branch(&self, name: &str) -> &Self {
        self.git(&["branch", name]);
        self
    }

    pub fn url(&self) -> String {
        self.dir.path().to_string_lossy().into_owned()
    }

    pub fn path(&self) -> &Path {
        self.dir.path()
    }

    /// Resolves a revision to a full commit sha.
    pub fn commit(&self, rev: &str) -> String {
        self.git(&["rev-parse", &format!("{rev}^{{commit}}")])
    }
}

#[cfg(unix)]
pub fn make_symlink(target: &str, link: &Path) {
    std::os::unix::fs::symlink(target, link).unwrap();
}

/// Windows makes a directory link or a file link; the target decides which.
#[cfg(windows)]
pub fn make_symlink(target: &str, link: &Path) {
    let resolved = link.parent().unwrap().join(target);
    if resolved.is_dir() {
        std::os::windows::fs::symlink_dir(target, link).unwrap();
    } else {
        std::os::windows::fs::symlink_file(target, link).unwrap();
    }
}

/// Runs git in `dir` with a hermetic configuration and returns trimmed stdout.
pub fn git_in(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .arg("-C")
        .arg(dir)
        .args(["-c", "user.name=Test", "-c", "user.email=test@example.com"])
        .args(["-c", "commit.gpgsign=false"])
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// Makes `dir` a git work tree (if it is not one yet) and commits everything in it.
pub fn commit_project(dir: &Path, message: &str) {
    if !dir.join(".git").exists() {
        git_in(dir, &["init", "-q", "-b", "main"]);
    }
    git_in(dir, &["add", "-A"]);
    git_in(dir, &["commit", "--allow-empty", "-q", "-m", message]);
}

/// The `source` a record text holds, parsed (TOML escapes Windows backslashes).
pub fn recorded_source(record: &str) -> String {
    let table: toml::Table = record.parse().unwrap();
    table["source"].as_str().unwrap().to_string()
}

/// The record text with its `source` line replaced by `source`.
pub fn with_source(record: &str, source: &str) -> String {
    let line = format!("source = {}", toml::Value::String(source.to_string()));
    record
        .lines()
        .map(|l| {
            if l.starts_with("source = ") {
                line.as_str()
            } else {
                l
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
        + "\n"
}

/// `canonicalize` as rootstock spells a local path: no Windows verbatim prefix.
pub fn canon(path: &Path) -> std::path::PathBuf {
    rootstock::source::canonical(path).unwrap()
}
