//! Test fixtures shared by the integration tests.
#![allow(dead_code)]

use std::fs;
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

    pub fn exec(&self, path: &str) -> &Self {
        let full = self.dir.path().join(path);
        fs::set_permissions(&full, fs::Permissions::from_mode(0o755)).unwrap();
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
