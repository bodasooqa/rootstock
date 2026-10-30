mod support;

use std::fs;
use std::path::{Path, PathBuf};

use assert_cmd::Command;
use predicates::prelude::*;
use support::TemplateRepo;
use tempfile::TempDir;

const CONFIG: &str = r#"schema = 1
root = "tpl"

[files]
starter = ["NOTES.md"]
"#;

fn rootstock() -> Command {
    let mut cmd = Command::cargo_bin("rootstock").unwrap();
    cmd.env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1");
    cmd
}

fn template() -> TemplateRepo {
    let repo = TemplateRepo::new();
    repo.file("rootstock.toml", CONFIG)
        .file("tpl/README.md", "readme\n")
        .file("tpl/NOTES.md", "notes\n")
        .tag("v1.0.0");
    repo
}

fn new_project(repo: &TemplateRepo) -> (TempDir, PathBuf) {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path().join("proj");
    rootstock()
        .args(["new", &repo.url()])
        .arg(&dir)
        .arg("--defaults")
        .assert()
        .code(0);
    (tmp, dir)
}

fn set_version(dir: &Path, version: &str) {
    let path = dir.join(".rootstock.toml");
    let text = fs::read_to_string(&path).unwrap();
    let new = text.replace("version = \"v1.0.0\"", &format!("version = \"{version}\""));
    assert_ne!(text, new);
    fs::write(&path, new).unwrap();
}

fn status(dir: &Path) -> Command {
    let mut cmd = rootstock();
    cmd.current_dir(dir).arg("status");
    cmd
}

fn check(dir: &Path) -> Command {
    let mut cmd = rootstock();
    cmd.current_dir(dir).arg("check");
    cmd
}

const CONFLICTED: &str = "a\n<<<<<<< project\nx\n=======\ny\n>>>>>>> template v2.0.0\nb\n";

#[test]
fn status_reports_major() {
    let repo = template();
    let (_tmp, dir) = new_project(&repo);
    repo.file("tpl/README.md", "v2\n").tag("v2.0.0");
    status(&dir)
        .assert()
        .code(0)
        .stdout("current v1.0.0\nlatest v2.0.0\nmajor true\n");
}

#[test]
fn status_minor_is_not_major() {
    let repo = template();
    let (_tmp, dir) = new_project(&repo);
    repo.file("tpl/README.md", "v1.1\n").tag("v1.1.0");
    status(&dir)
        .assert()
        .code(0)
        .stdout("current v1.0.0\nlatest v1.1.0\nmajor false\n");
}

#[test]
fn status_json() {
    let repo = template();
    let (_tmp, dir) = new_project(&repo);
    repo.file("tpl/README.md", "v2\n").tag("v2.0.0");
    status(&dir)
        .args(["--format", "json"])
        .assert()
        .code(0)
        .stdout("{\"current\":\"v1.0.0\",\"latest\":\"v2.0.0\",\"major\":true}\n");
}

#[test]
fn status_describe_version() {
    let repo = template();
    let (_tmp, dir) = new_project(&repo);
    repo.file("tpl/README.md", "v2\n").tag("v2.0.0");
    set_version(&dir, "v1.0.0-3-gabc1234");
    status(&dir)
        .assert()
        .code(0)
        .stdout("current v1.0.0-3-gabc1234\nlatest v2.0.0\nmajor true\n");
}

#[test]
fn status_not_a_release_exits_1() {
    let repo = template();
    let (_tmp, dir) = new_project(&repo);
    set_version(&dir, "HEAD");
    status(&dir)
        .assert()
        .code(1)
        .stdout("")
        .stderr(predicate::str::contains("`HEAD` is not a release"));
}

#[test]
fn status_without_record_exits_1() {
    let tmp = TempDir::new().unwrap();
    status(tmp.path())
        .assert()
        .code(1)
        .stderr(predicate::str::contains("not a rootstock project"));
}

#[test]
fn check_clean_exits_0() {
    let repo = template();
    let (_tmp, dir) = new_project(&repo);
    check(&dir).assert().code(0).stdout("");
}

#[test]
fn check_markers_exits_1() {
    let repo = template();
    let (_tmp, dir) = new_project(&repo);
    fs::write(dir.join("README.md"), CONFLICTED).unwrap();
    check(&dir)
        .assert()
        .code(1)
        .stdout("markers README.md\n")
        .stderr(predicate::str::contains("check failed: 1 problem(s)"));
}

#[test]
fn check_side_file_exits_1() {
    let repo = template();
    let (_tmp, dir) = new_project(&repo);
    fs::write(dir.join("README.md.rootstock-new"), [0u8, 1]).unwrap();
    fs::write(dir.join("NOTES.md.rootstock-new"), [0u8, 1]).unwrap();
    check(&dir)
        .assert()
        .code(1)
        .stdout("side-file NOTES.md.rootstock-new\nside-file README.md.rootstock-new\n")
        .stderr(predicate::str::contains("check failed: 2 problem(s)"));
}

#[test]
fn check_ignores_starter_markers() {
    let repo = template();
    let (_tmp, dir) = new_project(&repo);
    fs::write(dir.join("NOTES.md"), CONFLICTED).unwrap();
    check(&dir).assert().code(0).stdout("");
}

#[test]
fn check_without_record_exits_1() {
    let tmp = TempDir::new().unwrap();
    check(tmp.path())
        .assert()
        .code(1)
        .stderr(predicate::str::contains("not a rootstock project"));
}

#[test]
fn check_unreadable_record_exits_1() {
    let repo = template();
    let (_tmp, dir) = new_project(&repo);
    fs::write(dir.join(".rootstock.toml"), "not = [valid").unwrap();
    check(&dir)
        .assert()
        .code(1)
        .stderr(predicate::str::contains(".rootstock.toml"));
}

#[test]
fn check_does_not_follow_symlinks() {
    let repo = template();
    let (_tmp, dir) = new_project(&repo);
    let outside = TempDir::new().unwrap();
    fs::write(outside.path().join("f"), CONFLICTED).unwrap();
    fs::remove_file(dir.join("README.md")).unwrap();
    std::os::unix::fs::symlink(outside.path().join("f"), dir.join("README.md")).unwrap();
    check(&dir).assert().code(0).stdout("");
}
