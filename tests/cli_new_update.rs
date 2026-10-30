mod support;

use std::fs;
use std::path::Path;

use assert_cmd::Command;
use predicates::prelude::*;
use support::{TemplateRepo, commit_project, git_in};
use tempfile::TempDir;

const CONFIG: &str = r#"schema = 1
root = "tpl"

[files]
starter = ["NOTES.md"]

[[questions]]
name = "name"
type = "str"
default = "demo"

[[questions]]
name = "rust"
type = "bool"
default = true
"#;

const README_V1: &str = "l1\nl2\nl3\nl4\nl5\nl6\nl7\nl8\n";
const README_V2: &str = "l1\nl2\nl3\nl4\nl5\nl6\nl7\nL8-template\n";

/// v1.0.0 of a small template: managed README and src, starter NOTES, rust-only file.
fn template_v1() -> TemplateRepo {
    let repo = TemplateRepo::new();
    repo.file("rootstock.toml", CONFIG)
        .file("tpl/README.md", README_V1)
        .file("tpl/src.txt.jinja", "name={{ name }}\n")
        .file("tpl/NOTES.md", "notes\n")
        .file("tpl/{% if rust %}rust.txt{% endif %}", "rust\n")
        .tag("v1.0.0");
    repo
}

fn rootstock() -> Command {
    let mut cmd = Command::cargo_bin("rootstock").unwrap();
    cmd.env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1");
    cmd
}

/// Runs `rootstock new` into a fresh project dir and commits the result.
fn new_project(repo: &TemplateRepo) -> (TempDir, std::path::PathBuf) {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path().join("proj");
    rootstock()
        .args(["new", &repo.url()])
        .arg(&dir)
        .arg("--defaults")
        .assert()
        .code(0);
    commit_project(&dir, "init");
    (tmp, dir)
}

fn update(dir: &Path) -> Command {
    let mut cmd = rootstock();
    cmd.current_dir(dir).arg("update");
    cmd
}

#[test]
fn new_creates_project_and_record() {
    let repo = template_v1();
    let (_tmp, dir) = new_project(&repo);
    assert_eq!(
        fs::read_to_string(dir.join("README.md")).unwrap(),
        README_V1
    );
    assert_eq!(
        fs::read_to_string(dir.join("src.txt")).unwrap(),
        "name=demo\n"
    );
    assert!(dir.join("rust.txt").exists());
    let record = fs::read_to_string(dir.join(".rootstock.toml")).unwrap();
    assert!(record.contains("version = \"v1.0.0\""), "{record}");
    assert!(record.contains(&repo.commit("v1.0.0")), "{record}");
}

#[test]
fn new_noninteractive_missing_answer_exits_1() {
    let repo = TemplateRepo::new();
    repo.file(
        "rootstock.toml",
        "schema = 1\nroot = \"tpl\"\n[[questions]]\nname = \"who\"\ntype = \"str\"\n",
    )
    .file("tpl/a.txt", "a\n")
    .tag("v1.0.0");
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path().join("p");
    rootstock()
        .args(["new", &repo.url()])
        .arg(&dir)
        .assert()
        .code(1)
        .stderr(predicate::str::contains("who"));
    assert!(!dir.exists(), "nothing may be written");
}

#[test]
fn new_append_on_plain_list_exits_1_with_hint() {
    let repo = TemplateRepo::new();
    repo.file("rootstock.toml", "schema = 1\nroot = \"tpl\"\n")
        .file(
            "tpl/a.txt.jinja",
            "{% set x = [1] %}{% set _ = x.append(2) %}\n",
        )
        .tag("v1.0.0");
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path().join("p");
    rootstock()
        .args(["new", &repo.url()])
        .arg(&dir)
        .arg("--defaults")
        .assert()
        .code(1)
        .stderr(predicate::str::contains("`append` works only on a list"));
    assert!(!dir.exists(), "nothing may be written");
}

#[test]
fn new_config_parse_error_printed_once() {
    let repo = TemplateRepo::new();
    repo.file("rootstock.toml", "schema = 1\nroot = [ \"tpl\"\n")
        .file("tpl/a.txt", "a\n")
        .tag("v1.0.0");
    let tmp = TempDir::new().unwrap();
    let out = rootstock()
        .args(["new", &repo.url()])
        .arg(tmp.path().join("p"))
        .arg("--defaults")
        .assert()
        .code(1)
        .get_output()
        .stderr
        .clone();
    let stderr = String::from_utf8_lossy(&out);
    assert!(stderr.contains("rootstock.toml"), "{stderr}");
    assert_eq!(stderr.matches("root = [ \"tpl\"").count(), 1, "{stderr}");
}

/// A template with a yaml question, so that an answer TOML cannot store is possible.
fn yaml_template() -> TemplateRepo {
    let repo = TemplateRepo::new();
    repo.file(
        "rootstock.toml",
        "schema = 1\nroot = \"tpl\"\n[[questions]]\nname = \"extra\"\ntype = \"yaml\"\ndefault = \"{}\"\n",
    )
    .file("tpl/a.txt", "a\n")
    .tag("v1.0.0");
    repo
}

#[test]
fn new_unstorable_answer_writes_nothing() {
    let repo = yaml_template();
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path().join("p");
    rootstock()
        .args(["new", &repo.url()])
        .arg(&dir)
        .args(["--defaults", "--data", "extra={a: null}"])
        .assert()
        .code(1)
        .stderr(predicate::str::contains("extra"));
    assert!(!dir.exists(), "nothing may be written");
}

#[test]
fn update_unstorable_answer_writes_nothing() {
    let repo = yaml_template();
    let (_tmp, dir) = new_project(&repo);
    repo.file("tpl/a.txt", "a2\n").tag("v1.1.0");
    update(&dir)
        .args(["--data", "extra={a: null}"])
        .assert()
        .code(1)
        .stderr(predicate::str::contains("extra"));
    assert_eq!(fs::read_to_string(dir.join("a.txt")).unwrap(), "a\n");
}

#[test]
fn new_git_init_failure_says_project_created() {
    let repo = TemplateRepo::new();
    repo.file(
        "rootstock.toml",
        "schema = 1\nroot = \"tpl\"\n[[hooks]]\nrun = \"printf x > .git\"\n",
    )
    .file("tpl/a.txt", "a\n")
    .tag("v1.0.0");
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path().join("p");
    rootstock()
        .args(["new", &repo.url()])
        .arg(&dir)
        .args(["--defaults", "--git", "--trust"])
        .assert()
        .code(1)
        .stderr(predicate::str::contains(
            "project created; git init failed: ",
        ));
    assert!(dir.join(".rootstock.toml").exists());
}

#[test]
fn new_refuses_existing_record() {
    let repo = template_v1();
    let (_tmp, dir) = new_project(&repo);
    rootstock()
        .args(["new", &repo.url()])
        .arg(&dir)
        .arg("--defaults")
        .assert()
        .code(1)
        .stderr(predicate::str::contains("already a rootstock project"));
}

#[test]
fn new_refuses_credentials_in_url() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path().join("p");
    for url in [
        "https://zq7SECRETtoken9@example.invalid/o/r.git",
        "https://user:zq7SECRETtoken9@example.invalid/o/r.git",
    ] {
        rootstock()
            .args(["new", url])
            .arg(&dir)
            .args(["--defaults", "--version", "v1.0.0"])
            .assert()
            .code(1)
            .stderr(predicate::str::contains(
                "template URL carries credentials; configure a git credential helper instead",
            ))
            .stderr(predicate::str::contains("zq7SECRETtoken9").not());
        assert!(!dir.exists());
    }
}

#[test]
fn update_refuses_credentials_in_recorded_source() {
    let repo = template_v1();
    let (_tmp, dir) = new_project(&repo);
    let record = fs::read_to_string(dir.join(".rootstock.toml")).unwrap();
    let edited = record.replace(
        &format!(
            "source = \"{}\"",
            repo.path().canonicalize().unwrap().display()
        ),
        "source = \"https://zq7SECRETtoken9@example.invalid/o/r.git\"",
    );
    assert_ne!(edited, record);
    fs::write(dir.join(".rootstock.toml"), edited).unwrap();
    commit_project(&dir, "edit source");
    update(&dir)
        .assert()
        .code(1)
        .stderr(predicate::str::contains("template URL carries credentials"))
        .stderr(predicate::str::contains("zq7SECRETtoken9").not());
}

#[test]
fn new_and_update_from_repo_subdirectory() {
    let repo = TemplateRepo::new();
    repo.file("README.md", "the repository, not the template\n")
        .file("tpl/rootstock.toml", CONFIG)
        .file("tpl/tpl/README.md", README_V1)
        .file("tpl/tpl/src.txt.jinja", "name={{ name }}\n")
        .file("tpl/tpl/NOTES.md", "notes\n")
        .tag("v1.0.0");
    let sub = repo.path().join("tpl");
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path().join("proj");
    rootstock()
        .args(["new", &sub.to_string_lossy()])
        .arg(&dir)
        .arg("--defaults")
        .assert()
        .code(0);
    assert_eq!(
        fs::read_to_string(dir.join("README.md")).unwrap(),
        README_V1
    );
    assert_eq!(
        fs::read_to_string(dir.join("src.txt")).unwrap(),
        "name=demo\n"
    );
    let record = fs::read_to_string(dir.join(".rootstock.toml")).unwrap();
    let recorded = sub.canonicalize().unwrap();
    assert!(
        record.contains(&format!("source = \"{}\"", recorded.display())),
        "{record}"
    );
    commit_project(&dir, "init");

    repo.file("tpl/tpl/README.md", README_V2)
        .file("README.md", "changed outside the template\n")
        .tag("v1.1.0");
    update(&dir).assert().code(0);
    assert_eq!(
        fs::read_to_string(dir.join("README.md")).unwrap(),
        README_V2
    );
    let record = fs::read_to_string(dir.join(".rootstock.toml")).unwrap();
    assert!(record.contains("version = \"v1.1.0\""), "{record}");
}

/// A repository whose `tpl/` holds only a Copier template, tagged v1.0.0.
fn copier_subdir_repo() -> TemplateRepo {
    let repo = TemplateRepo::new();
    repo.file("tpl/copier.yml", "_subdirectory: tpl\n")
        .file("tpl/tpl/README.md", README_V1)
        .tag("v1.0.0");
    repo
}

#[test]
fn new_from_a_copier_only_subdirectory_hints_import_copier() {
    let repo = copier_subdir_repo();
    let tmp = TempDir::new().unwrap();
    let hint = predicate::str::contains("run `rootstock import-copier`");
    // The version in the message is the tag the user knows, not a sha.
    let named = predicate::str::contains("template version v1.0.0 has copier.yml");
    rootstock()
        .args(["new", &repo.path().join("tpl").to_string_lossy()])
        .arg(tmp.path().join("a"))
        .arg("--defaults")
        .assert()
        .code(1)
        .stderr(hint.clone())
        .stderr(named.clone());
    rootstock()
        .args(["new", &repo.url(), "--subdir", "tpl"])
        .arg(tmp.path().join("b"))
        .arg("--defaults")
        .assert()
        .code(1)
        .stderr(hint)
        .stderr(named);
}

#[test]
fn update_converts_a_copier_only_subdirectory_in_memory() {
    let repo = TemplateRepo::new();
    repo.file("tpl/rootstock.toml", "schema = 1\nroot = \"tpl\"\n")
        .file("tpl/tpl/README.md", README_V1)
        .tag("v1.0.0");
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path().join("proj");
    rootstock()
        .args(["new", &repo.url(), "--subdir", "tpl"])
        .arg(&dir)
        .arg("--defaults")
        .assert()
        .code(0);
    commit_project(&dir, "init");
    repo.remove("tpl/rootstock.toml")
        .file("tpl/copier.yml", "_subdirectory: tpl\n")
        .file("tpl/tpl/README.md", README_V2)
        .tag("v1.1.0");
    update(&dir).assert().code(0);
    assert_eq!(
        fs::read_to_string(dir.join("README.md")).unwrap(),
        README_V2
    );
    let record = fs::read_to_string(dir.join(".rootstock.toml")).unwrap();
    assert!(record.contains("version = \"v1.1.0\""), "{record}");
    assert!(record.contains("subdir = \"tpl\""), "{record}");
}

#[cfg(unix)]
#[test]
fn update_fails_cleanly_when_git_merge_file_fails() {
    use std::os::unix::fs::PermissionsExt;
    let (_repo, _tmp, dir) = conflicting_project();
    let before_readme = fs::read(dir.join("README.md")).unwrap();
    let before_record = fs::read(dir.join(".rootstock.toml")).unwrap();
    let real = std::process::Command::new("sh")
        .args(["-c", "command -v git"])
        .output()
        .unwrap();
    let real = String::from_utf8(real.stdout).unwrap().trim().to_string();
    let bin = TempDir::new().unwrap();
    let wrapper = bin.path().join("git");
    fs::write(
        &wrapper,
        format!(
            "#!/bin/sh\nfor a in \"$@\"; do [ \"$a\" = merge-file ] && {{ echo boom >&2; exit 255; }}; done\nexec {real} \"$@\"\n"
        ),
    )
    .unwrap();
    fs::set_permissions(&wrapper, fs::Permissions::from_mode(0o755)).unwrap();
    let path = format!(
        "{}:{}",
        bin.path().display(),
        std::env::var("PATH").unwrap()
    );
    update(&dir)
        .env("PATH", path)
        .assert()
        .code(1)
        .stderr(predicate::str::contains("`README.md`"))
        .stderr(predicate::str::contains("exit 255"));
    assert_eq!(fs::read(dir.join("README.md")).unwrap(), before_readme);
    assert_eq!(
        fs::read(dir.join(".rootstock.toml")).unwrap(),
        before_record
    );
    assert_eq!(git_in(&dir, &["status", "--porcelain"]), "");
}

#[cfg(unix)]
#[test]
fn subdirectory_template_root_cannot_leave_the_template_dir() {
    let repo = TemplateRepo::new();
    repo.file("shared/README.md", "outside the template\n")
        .file("tpl/rootstock.toml", "schema = 1\nroot = \"files\"\n")
        .symlink("tpl/files", "../shared")
        .tag("v1.0.0");
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path().join("proj");
    rootstock()
        .args(["new", &repo.path().join("tpl").to_string_lossy()])
        .arg(&dir)
        .arg("--defaults")
        .assert()
        .code(1)
        .stderr(predicate::str::contains("resolves outside"));
    assert!(!dir.exists());
}

const VERSION_RULE: &str = "version must be a tag vX.Y.Z (or HEAD for a local template)";

#[test]
fn new_rejects_non_semver_version() {
    let repo = template_v1();
    repo.branch("release");
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path().join("p");
    for version in ["main", "foo", "release", "v1.0", "1.0.0", "v01.0.0"] {
        rootstock()
            .args(["new", &repo.url()])
            .arg(&dir)
            .args(["--defaults", "--version", version])
            .assert()
            .code(1)
            .stderr(predicate::str::contains(VERSION_RULE));
        assert!(!dir.exists(), "{version}");
    }
    // HEAD only names the working tree of a local template.
    let url = format!("file://{}", repo.path().display());
    rootstock()
        .args(["new", &url])
        .arg(&dir)
        .args(["--defaults", "--version", "HEAD"])
        .assert()
        .code(1)
        .stderr(predicate::str::contains(VERSION_RULE));
    assert!(!dir.exists());
    // A pre-release tag is a version.
    repo.file("tpl/README.md", README_V2).tag("v1.1.0-rc.1");
    rootstock()
        .args(["new", &repo.url()])
        .arg(&dir)
        .args(["--defaults", "--version", "v1.1.0-rc.1"])
        .assert()
        .code(0);
}

#[test]
fn update_rejects_non_semver_to() {
    let repo = template_v1();
    let (_tmp, dir) = new_project(&repo);
    repo.file("tpl/README.md", README_V2).tag("v1.1.0");
    repo.branch("next");
    for to in ["main", "next", "v1.1"] {
        update(&dir)
            .args(["--to", to])
            .assert()
            .code(1)
            .stderr(predicate::str::contains(VERSION_RULE));
        assert_untouched(&dir);
    }
}

#[test]
fn new_git_requires_empty_dir() {
    let repo = template_v1();
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path().join("p");
    fs::create_dir(&dir).unwrap();
    fs::write(dir.join("x"), "x").unwrap();
    rootstock()
        .args(["new", &repo.url()])
        .arg(&dir)
        .args(["--defaults", "--git"])
        .assert()
        .code(1)
        .stderr(predicate::str::contains("--git needs an empty directory"));
    assert!(!dir.join("README.md").exists());

    let empty = tmp.path().join("fresh");
    rootstock()
        .args(["new", &repo.url()])
        .arg(&empty)
        .args(["--defaults", "--git"])
        .assert()
        .code(0);
    assert!(empty.join(".git").exists());
}

#[test]
fn new_without_git_flag_does_not_init() {
    let repo = template_v1();
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path().join("p");
    rootstock()
        .args(["new", &repo.url()])
        .arg(&dir)
        .arg("--defaults")
        .assert()
        .code(0);
    assert!(!dir.join(".git").exists());
}

#[test]
fn new_local_source_recorded_absolute() {
    let repo = template_v1();
    let tmp = TempDir::new().unwrap();
    let rel = pathdiff(&repo.url(), tmp.path());
    rootstock()
        .current_dir(tmp.path())
        .args(["new", &rel, "p", "--defaults"])
        .assert()
        .code(0);
    let record = fs::read_to_string(tmp.path().join("p/.rootstock.toml")).unwrap();
    let canonical = repo.path().canonicalize().unwrap();
    assert!(
        record.contains(&format!("source = \"{}\"", canonical.display())),
        "{record}"
    );
}

/// A relative path from `from` to `to` (both absolute) through `..`.
fn pathdiff(to: &str, from: &Path) -> String {
    let to = Path::new(to).canonicalize().unwrap();
    let from = from.canonicalize().unwrap();
    let up = from.components().count() - 1;
    let mut out = "../".repeat(up);
    out.push_str(to.to_string_lossy().trim_start_matches('/'));
    out
}

#[test]
fn update_clean_exits_0_and_records_version() {
    let repo = template_v1();
    let (_tmp, dir) = new_project(&repo);
    repo.file("tpl/README.md", README_V2).tag("v1.1.0");
    update(&dir)
        .assert()
        .code(0)
        .stdout(predicate::str::is_empty())
        .stderr(predicate::str::contains("updated to v1.1.0"));
    assert_eq!(
        fs::read_to_string(dir.join("README.md")).unwrap(),
        README_V2
    );
    let record = fs::read_to_string(dir.join(".rootstock.toml")).unwrap();
    assert!(record.contains("version = \"v1.1.0\""), "{record}");
    assert!(record.contains(&repo.commit("v1.1.0")), "{record}");
}

#[test]
fn update_keeps_project_edit() {
    let repo = template_v1();
    let (_tmp, dir) = new_project(&repo);
    fs::write(
        dir.join("README.md"),
        "L1-project\nl2\nl3\nl4\nl5\nl6\nl7\nl8\n",
    )
    .unwrap();
    commit_project(&dir, "edit");
    repo.file("tpl/README.md", README_V2).tag("v1.1.0");
    update(&dir).assert().code(0);
    assert_eq!(
        fs::read_to_string(dir.join("README.md")).unwrap(),
        "L1-project\nl2\nl3\nl4\nl5\nl6\nl7\nL8-template\n"
    );
}

#[test]
fn update_overlap_exits_2_lists_markers() {
    let repo = template_v1();
    let (_tmp, dir) = new_project(&repo);
    fs::write(
        dir.join("README.md"),
        "l1\nl2\nl3\nl4\nl5\nl6\nl7\nL8-project\n",
    )
    .unwrap();
    commit_project(&dir, "edit");
    repo.file("tpl/README.md", README_V2).tag("v1.1.0");
    update(&dir).assert().code(2).stdout("markers README.md\n");
    let text = fs::read_to_string(dir.join("README.md")).unwrap();
    assert!(text.contains("<<<<<<< project"), "{text}");
    assert!(text.contains(">>>>>>> template v1.1.0"), "{text}");
}

#[test]
fn update_conflict_ignores_the_users_git_conflict_style() {
    let repo = template_v1();
    let (_tmp, dir) = new_project(&repo);
    fs::write(
        dir.join("README.md"),
        "l1\nl2\nl3\nl4\nl5\nl6\nl7\nL8-project\n",
    )
    .unwrap();
    commit_project(&dir, "edit");
    repo.file("tpl/README.md", README_V2).tag("v1.1.0");
    let cfg_dir = TempDir::new().unwrap();
    let cfg = cfg_dir.path().join("gitconfig");
    fs::write(&cfg, "[merge]\n\tconflictStyle = diff3\n").unwrap();
    update(&dir).env("GIT_CONFIG_GLOBAL", &cfg).assert().code(2);
    let text = fs::read_to_string(dir.join("README.md")).unwrap();
    assert!(!text.contains("|||||||"), "{text}");
    assert!(text.contains("<<<<<<< project"), "{text}");
}

/// A project whose README conflicts on update to v1.1.0.
fn conflicting_project() -> (TemplateRepo, TempDir, std::path::PathBuf) {
    let repo = template_v1();
    let (tmp, dir) = new_project(&repo);
    fs::write(
        dir.join("README.md"),
        "l1\nl2\nl3\nl4\nl5\nl6\nl7\nL8-project\n",
    )
    .unwrap();
    commit_project(&dir, "edit");
    repo.file("tpl/README.md", README_V2).tag("v1.1.0");
    (repo, tmp, dir)
}

#[test]
fn update_ignores_inherited_git_location_variables() {
    let (_repo, _tmp, dir) = conflicting_project();
    update(&dir)
        .env("GIT_DIR", dir.join(".git"))
        .env("GIT_WORK_TREE", &dir)
        .assert()
        .code(2)
        .stdout("markers README.md\n");
    let text = fs::read_to_string(dir.join("README.md")).unwrap();
    assert!(text.contains("<<<<<<< project"), "{text}");
}

#[test]
fn update_conflict_ignores_the_config_of_the_repo_around_the_temp_dir() {
    let (_repo, _tmp, dir) = conflicting_project();
    let outer = TempDir::new().unwrap();
    git_in(outer.path(), &["init", "-q"]);
    git_in(outer.path(), &["config", "merge.conflictStyle", "diff3"]);
    let scratch = outer.path().join("scratch");
    fs::create_dir(&scratch).unwrap();
    update(&dir).env("TMPDIR", &scratch).assert().code(2);
    let text = fs::read_to_string(dir.join("README.md")).unwrap();
    assert!(!text.contains("|||||||"), "{text}");
    assert!(text.contains("<<<<<<< project"), "{text}");
}

#[test]
fn update_json_output() {
    let repo = template_v1();
    let (_tmp, dir) = new_project(&repo);
    fs::write(
        dir.join("README.md"),
        "l1\nl2\nl3\nl4\nl5\nl6\nl7\nL8-project\n",
    )
    .unwrap();
    commit_project(&dir, "edit");
    repo.file("tpl/README.md", README_V2).tag("v1.1.0");
    let out = update(&dir)
        .args(["--format", "json"])
        .assert()
        .code(2)
        .get_output()
        .stdout
        .clone();
    let value: serde_json::Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(
        value,
        serde_json::json!({
            "version": "v1.1.0",
            "conflicts": [{"path": "README.md", "kind": "markers"}]
        })
    );
}

#[test]
fn update_does_not_recreate_deleted_starter() {
    let repo = template_v1();
    let (_tmp, dir) = new_project(&repo);
    fs::remove_file(dir.join("NOTES.md")).unwrap();
    commit_project(&dir, "rm notes");
    repo.file("tpl/src.txt.jinja", "name={{ name }}!\n")
        .tag("v1.1.0");
    update(&dir).assert().code(0);
    assert!(!dir.join("NOTES.md").exists());
    assert_eq!(
        fs::read_to_string(dir.join("src.txt")).unwrap(),
        "name=demo!\n"
    );
}

#[test]
fn update_removes_file_dropped_by_template() {
    let repo = template_v1();
    let (_tmp, dir) = new_project(&repo);
    repo.remove("tpl/src.txt.jinja").tag("v1.1.0");
    update(&dir).assert().code(0);
    assert!(!dir.join("src.txt").exists());
    let record = fs::read_to_string(dir.join(".rootstock.toml")).unwrap();
    assert!(!record.contains("src.txt"), "{record}");
}

#[test]
fn update_dirty_tree_exits_1() {
    let repo = template_v1();
    let (_tmp, dir) = new_project(&repo);
    repo.file("tpl/README.md", README_V2).tag("v1.1.0");
    fs::write(dir.join("stray.txt"), "x").unwrap();
    update(&dir)
        .assert()
        .code(1)
        .stderr(predicate::str::contains("uncommitted changes"));
    assert_eq!(
        fs::read_to_string(dir.join("README.md")).unwrap(),
        README_V1
    );
}

#[test]
fn update_outside_git_work_tree_exits_1() {
    let repo = template_v1();
    let (_tmp, dir) = new_project(&repo);
    fs::remove_dir_all(dir.join(".git")).unwrap();
    repo.file("tpl/README.md", README_V2).tag("v1.1.0");
    update(&dir)
        .env("GIT_CEILING_DIRECTORIES", dir.parent().unwrap())
        .assert()
        .code(1)
        .stderr(predicate::str::contains("not a git work tree"));
}

#[test]
fn update_with_leftover_markers_exits_1() {
    let repo = template_v1();
    let (_tmp, dir) = new_project(&repo);
    fs::write(
        dir.join("README.md"),
        "<<<<<<< project\nmine\n=======\ntheirs\n>>>>>>> template v1.0.0\n",
    )
    .unwrap();
    commit_project(&dir, "markers");
    repo.file("tpl/README.md", README_V2).tag("v1.1.0");
    update(&dir)
        .assert()
        .code(1)
        .stderr(predicate::str::contains("unresolved conflict markers"))
        .stderr(predicate::str::contains("README.md"));
    assert_eq!(
        fs::read(dir.join("README.md")).unwrap(),
        b"<<<<<<< project\nmine\n=======\ntheirs\n>>>>>>> template v1.0.0\n"
    );
    let record = fs::read_to_string(dir.join(".rootstock.toml")).unwrap();
    assert!(record.contains("version = \"v1.0.0\""), "{record}");
}

#[test]
fn update_missing_base_commit_exits_1_writes_nothing() {
    let repo = template_v1();
    let (_tmp, dir) = new_project(&repo);
    let old = repo.commit("v1.0.0");
    // Rewrite history so that v1.0.0's commit becomes unreachable, then prune it.
    repo.git(&["tag", "-d", "v1.0.0"]);
    repo.git(&["checkout", "-q", "--orphan", "rewritten"]);
    repo.file("tpl/README.md", README_V2);
    repo.tag("v1.1.0");
    repo.git(&["branch", "-D", "main"]);
    repo.git(&["reflog", "expire", "--expire=now", "--all"]);
    repo.git(&["gc", "--prune=now", "-q"]);
    let gone = std::process::Command::new("git")
        .arg("-C")
        .arg(repo.path())
        .args(["cat-file", "-e", &old])
        .output()
        .unwrap();
    assert!(!gone.status.success(), "old commit must be pruned");

    let before_readme = fs::read(dir.join("README.md")).unwrap();
    let before_record = fs::read(dir.join(".rootstock.toml")).unwrap();
    update(&dir)
        .assert()
        .code(1)
        .stderr(predicate::str::contains(&old));
    assert_eq!(fs::read(dir.join("README.md")).unwrap(), before_readme);
    assert_eq!(
        fs::read(dir.join(".rootstock.toml")).unwrap(),
        before_record
    );
    assert!(git_in(&dir, &["status", "--porcelain"]).is_empty());
}

#[test]
fn update_data_changes_answer() {
    let repo = template_v1();
    let (_tmp, dir) = new_project(&repo);
    repo.file("tpl/README.md", README_V2).tag("v1.1.0");
    update(&dir).args(["--data", "rust=false"]).assert().code(0);
    assert!(!dir.join("rust.txt").exists());
    let record = fs::read_to_string(dir.join(".rootstock.toml")).unwrap();
    assert!(record.contains("rust = false"), "{record}");
}

#[test]
fn update_to_specific_tag() {
    let repo = template_v1();
    let (_tmp, dir) = new_project(&repo);
    repo.file("tpl/README.md", README_V2).tag("v1.1.0");
    repo.file("tpl/src.txt.jinja", "later\n").tag("v1.2.0");
    update(&dir).args(["--to", "v1.1.0"]).assert().code(0);
    assert_eq!(
        fs::read_to_string(dir.join("src.txt")).unwrap(),
        "name=demo\n"
    );
    let record = fs::read_to_string(dir.join(".rootstock.toml")).unwrap();
    assert!(record.contains("version = \"v1.1.0\""), "{record}");
}

#[test]
fn update_already_at_target_exits_0() {
    let repo = template_v1();
    let (_tmp, dir) = new_project(&repo);
    update(&dir)
        .assert()
        .code(0)
        .stdout(predicate::str::is_empty())
        .stderr(predicate::str::contains("already at v1.0.0"));
}

#[test]
fn update_data_file_is_merged_under_data() {
    let repo = template_v1();
    let (tmp, dir) = new_project(&repo);
    repo.file("tpl/README.md", README_V2).tag("v1.1.0");
    let file = tmp.path().join("answers.toml");
    fs::write(&file, "name = \"fromfile\"\nrust = false\n").unwrap();
    update(&dir)
        .arg("--data-file")
        .arg(&file)
        .args(["--data", "rust=true"])
        .assert()
        .code(0);
    assert_eq!(
        fs::read_to_string(dir.join("src.txt")).unwrap(),
        "name=fromfile\n"
    );
    assert!(dir.join("rust.txt").exists());

    let bad = tmp.path().join("bad.toml");
    fs::write(&bad, "nope = 1\n").unwrap();
    commit_project(&dir, "after update");
    repo.file("tpl/README.md", "x\n").tag("v1.2.0");
    update(&dir)
        .arg("--data-file")
        .arg(&bad)
        .assert()
        .code(1)
        .stderr(predicate::str::contains("nope"));
}

#[test]
fn update_hooks_need_trust() {
    let repo = template_v1();
    let (_tmp, dir) = new_project(&repo);
    let config = format!("{CONFIG}\n[[hooks]]\nrun = \"touch hooked.txt\"\nwhen = [\"update\"]\n");
    repo.file("rootstock.toml", &config)
        .file("tpl/README.md", README_V2)
        .tag("v1.1.0");
    update(&dir)
        .assert()
        .code(1)
        .stderr(predicate::str::contains("--trust"));
    assert_eq!(
        fs::read_to_string(dir.join("README.md")).unwrap(),
        README_V1
    );
    assert!(!dir.join("hooked.txt").exists());
    let record = fs::read_to_string(dir.join(".rootstock.toml")).unwrap();
    assert!(record.contains("version = \"v1.0.0\""), "{record}");

    update(&dir).arg("--trust").assert().code(0);
    assert!(dir.join("hooked.txt").exists());
}

#[test]
fn update_hook_failure_reports_record_not_updated() {
    let repo = template_v1();
    let (_tmp, dir) = new_project(&repo);
    let config = format!("{CONFIG}\n[[hooks]]\nrun = \"exit 3\"\nwhen = [\"update\"]\n");
    repo.file("rootstock.toml", &config)
        .file("tpl/README.md", README_V2)
        .tag("v1.1.0");
    update(&dir)
        .arg("--trust")
        .assert()
        .code(1)
        .stderr(predicate::str::contains(
            "hooks failed; the record was not updated",
        ));
    let record = fs::read_to_string(dir.join(".rootstock.toml")).unwrap();
    assert!(record.contains("version = \"v1.0.0\""), "{record}");
}

#[test]
fn update_hook_failure_still_lists_conflicts() {
    let repo = template_v1();
    let (_tmp, dir) = new_project(&repo);
    fs::write(
        dir.join("README.md"),
        "l1\nl2\nl3\nl4\nl5\nl6\nl7\nL8-project\n",
    )
    .unwrap();
    commit_project(&dir, "edit");
    let config = format!("{CONFIG}\n[[hooks]]\nrun = \"exit 3\"\nwhen = [\"update\"]\n");
    repo.file("rootstock.toml", &config)
        .file("tpl/README.md", README_V2)
        .tag("v1.1.0");
    update(&dir)
        .arg("--trust")
        .assert()
        .code(1)
        .stdout("markers README.md\n")
        .stderr(predicate::str::contains("hooks failed"));
}

#[test]
fn new_hook_failure_still_lists_conflicts_as_json() {
    let repo = TemplateRepo::new();
    repo.file(
        "rootstock.toml",
        "schema = 1\nroot = \"tpl\"\n\n[[hooks]]\nrun = \"exit 3\"\nwhen = [\"new\"]\n",
    )
    .file("tpl/a.txt", "template\n")
    .tag("v1.0.0");
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path().join("p");
    fs::create_dir(&dir).unwrap();
    fs::write(dir.join("a.txt"), "mine\n").unwrap();
    rootstock()
        .args(["new", &repo.url()])
        .arg(&dir)
        .args(["--defaults", "--trust", "--format", "json"])
        .assert()
        .code(1)
        .stdout(predicate::str::contains(r#""kind":"added-by-both""#))
        .stdout(predicate::str::contains(r#""path":"a.txt""#))
        .stderr(predicate::str::contains("hooks failed"));
}

#[test]
fn update_scoped_to_project_dir() {
    let repo = template_v1();
    let tmp = TempDir::new().unwrap();
    let mono = tmp.path().join("mono");
    let dir = mono.join("proj");
    fs::create_dir_all(&dir).unwrap();
    rootstock()
        .args(["new", &repo.url()])
        .arg(&dir)
        .arg("--defaults")
        .assert()
        .code(0);
    commit_project(&mono, "init");
    fs::create_dir(mono.join("other")).unwrap();
    fs::write(mono.join("other/uncommitted.txt"), "x").unwrap();
    repo.file("tpl/README.md", README_V2).tag("v1.1.0");
    update(&dir).assert().code(0);
    assert_eq!(
        fs::read_to_string(dir.join("README.md")).unwrap(),
        README_V2
    );
}

#[test]
fn update_json_clean_and_already_at() {
    let repo = template_v1();
    let (_tmp, dir) = new_project(&repo);
    let expect = |version: &str| serde_json::json!({"version": version, "conflicts": []});

    let out = update(&dir)
        .args(["--format", "json"])
        .assert()
        .code(0)
        .get_output()
        .stdout
        .clone();
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&out).unwrap(),
        expect("v1.0.0")
    );

    repo.file("tpl/README.md", README_V2).tag("v1.1.0");
    let out = update(&dir)
        .args(["--format", "json"])
        .assert()
        .code(0)
        .get_output()
        .stdout
        .clone();
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&out).unwrap(),
        expect("v1.1.0")
    );
}

/// The `commit = "..."` line of the project record.
fn record_commit_line(dir: &Path) -> String {
    let record = fs::read_to_string(dir.join(".rootstock.toml")).unwrap();
    record
        .lines()
        .find(|l| l.starts_with("commit = "))
        .unwrap()
        .to_string()
}

#[test]
fn update_data_on_current_version_changes_answer() {
    let repo = template_v1();
    let (_tmp, dir) = new_project(&repo);
    let commit_before = record_commit_line(&dir);
    update(&dir)
        .args(["--data", "name=new"])
        .assert()
        .code(0)
        .stdout(predicate::str::is_empty());
    assert_eq!(
        fs::read_to_string(dir.join("src.txt")).unwrap(),
        "name=new\n"
    );
    let record = fs::read_to_string(dir.join(".rootstock.toml")).unwrap();
    assert!(record.contains("name = \"new\""), "{record}");
    assert!(record.contains("version = \"v1.0.0\""), "{record}");
    assert_eq!(record_commit_line(&dir), commit_before);
}

#[test]
fn update_data_equal_to_recorded_answer_changes_nothing() {
    let repo = template_v1();
    let (_tmp, dir) = new_project(&repo);
    let record_before = fs::read(dir.join(".rootstock.toml")).unwrap();
    let src_before = fs::read(dir.join("src.txt")).unwrap();
    update(&dir)
        .args(["--data", "name=demo"])
        .assert()
        .code(0)
        .stdout(predicate::str::is_empty())
        .stderr(predicate::str::contains(
            "already at v1.0.0; answers unchanged",
        ));
    assert_eq!(
        fs::read(dir.join(".rootstock.toml")).unwrap(),
        record_before
    );
    assert_eq!(fs::read(dir.join("src.txt")).unwrap(), src_before);
    assert!(git_in(&dir, &["status", "--porcelain"]).is_empty());
}

#[test]
fn update_data_on_current_version_removes_disabled_file() {
    let repo = template_v1();
    let (_tmp, dir) = new_project(&repo);
    assert!(dir.join("rust.txt").exists());
    let commit_before = record_commit_line(&dir);
    update(&dir).args(["--data", "rust=false"]).assert().code(0);
    assert!(!dir.join("rust.txt").exists());
    let record = fs::read_to_string(dir.join(".rootstock.toml")).unwrap();
    assert!(record.contains("rust = false"), "{record}");
    assert!(record.contains("version = \"v1.0.0\""), "{record}");
    assert_eq!(record_commit_line(&dir), commit_before);
}

#[test]
fn update_data_on_current_version_conflicts_with_project_edit() {
    let repo = template_v1();
    let (_tmp, dir) = new_project(&repo);
    fs::write(dir.join("src.txt"), "name=mine\n").unwrap();
    commit_project(&dir, "edit");
    let commit_before = record_commit_line(&dir);
    update(&dir)
        .args(["--data", "name=new"])
        .assert()
        .code(2)
        .stdout(predicate::str::contains("src.txt"));
    let text = fs::read_to_string(dir.join("src.txt")).unwrap();
    assert!(text.contains("<<<<<<< project"), "{text}");
    let record = fs::read_to_string(dir.join(".rootstock.toml")).unwrap();
    assert!(record.contains("name = \"new\""), "{record}");
    assert_eq!(record_commit_line(&dir), commit_before);
}

#[test]
fn update_data_on_a_moved_tag_keeps_the_recorded_commit() {
    let repo = template_v1();
    let (_tmp, dir) = new_project(&repo);
    let commit_before = record_commit_line(&dir);
    // The tag of the current version is moved to a commit with other content.
    repo.file("tpl/README.md", README_V2)
        .file("tpl/src.txt.jinja", "name={{ name }}!\n");
    repo.git(&["add", "-A"]);
    repo.git(&["commit", "-q", "-m", "moved"]);
    repo.git(&["tag", "-f", "v1.0.0"]);
    update(&dir).args(["--data", "name=new"]).assert().code(0);
    assert_eq!(
        fs::read_to_string(dir.join("src.txt")).unwrap(),
        "name=new\n"
    );
    assert_eq!(
        fs::read_to_string(dir.join("README.md")).unwrap(),
        README_V1
    );
    assert_eq!(record_commit_line(&dir), commit_before);
}

#[test]
fn update_hooks_do_not_run_when_the_answer_is_unchanged() {
    let repo = TemplateRepo::new();
    let config = format!("{CONFIG}\n[[hooks]]\nrun = \"touch hooked.txt\"\nwhen = [\"update\"]\n");
    repo.file("rootstock.toml", &config)
        .file("tpl/README.md", README_V1)
        .file("tpl/src.txt.jinja", "name={{ name }}\n")
        .tag("v1.0.0");
    let (_tmp, dir) = new_project(&repo);
    let record_before = fs::read(dir.join(".rootstock.toml")).unwrap();
    let src_before = fs::read(dir.join("src.txt")).unwrap();
    update(&dir)
        .args(["--data", "name=demo", "--trust"])
        .assert()
        .code(0)
        .stderr(predicate::str::contains("answers unchanged"));
    assert!(!dir.join("hooked.txt").exists());
    assert_eq!(
        fs::read(dir.join(".rootstock.toml")).unwrap(),
        record_before
    );
    assert_eq!(fs::read(dir.join("src.txt")).unwrap(), src_before);
}

#[test]
fn new_head_records_head() {
    let repo = template_v1();
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path().join("p");
    rootstock()
        .args(["new", &repo.url()])
        .arg(&dir)
        .args(["--version", "HEAD", "--defaults"])
        .assert()
        .code(0);
    let record = fs::read_to_string(dir.join(".rootstock.toml")).unwrap();
    assert!(record.contains("version = \"HEAD\""), "{record}");
    assert!(record.contains(&repo.commit("HEAD")), "{record}");
}

#[test]
fn update_to_head_always_runs() {
    let repo = template_v1();
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path().join("p");
    rootstock()
        .args(["new", &repo.url()])
        .arg(&dir)
        .args(["--version", "HEAD", "--defaults"])
        .assert()
        .code(0);
    commit_project(&dir, "init");
    repo.file("tpl/README.md", README_V2);
    repo.git(&["commit", "-q", "-am", "untagged change"]);
    update(&dir)
        .args(["--to", "HEAD"])
        .assert()
        .code(0)
        .stderr(predicate::str::contains("updated to HEAD"));
    assert_eq!(
        fs::read_to_string(dir.join("README.md")).unwrap(),
        README_V2
    );
    let record = fs::read_to_string(dir.join(".rootstock.toml")).unwrap();
    assert!(record.contains(&repo.commit("HEAD")), "{record}");
}

#[test]
fn conflict_count_pluralized() {
    let repo = template_v1();
    let (_tmp, dir) = new_project(&repo);
    fs::write(
        dir.join("README.md"),
        "l1\nl2\nl3\nl4\nl5\nl6\nl7\nL8-project\n",
    )
    .unwrap();
    commit_project(&dir, "edit");
    repo.file("tpl/README.md", README_V2).tag("v1.1.0");
    update(&dir)
        .assert()
        .code(2)
        .stderr(predicate::str::contains("\n1 conflict\n"));

    let (_tmp2, dir) = new_project(&repo);
    fs::write(
        dir.join("README.md"),
        "l1\nl2\nl3\nl4\nl5\nl6\nl7\nL8-project\n",
    )
    .unwrap();
    fs::write(dir.join("src.txt"), "mine\n").unwrap();
    commit_project(&dir, "edit");
    repo.file("tpl/README.md", "l1\nl2\nl3\nl4\nl5\nl6\nl7\nL8-v3\n")
        .file("tpl/src.txt.jinja", "name={{ name }} v3\n")
        .tag("v1.2.0");
    update(&dir)
        .args(["--to", "v1.2.0"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("\n2 conflicts\n"));
}

#[test]
fn new_stdout_empty() {
    let repo = template_v1();
    let tmp = TempDir::new().unwrap();
    rootstock()
        .args(["new", &repo.url()])
        .arg(tmp.path().join("p"))
        .arg("--defaults")
        .assert()
        .code(0)
        .stdout(predicate::str::is_empty());
}

const LEFTOVER: &str = "<<<<<<< project\nmine\n=======\ntheirs\n>>>>>>> template v1.0.0\n";

/// Appends a managed inventory entry to the project record.
fn add_managed_entry(dir: &Path, path: &str) {
    let mut record = fs::read_to_string(dir.join(".rootstock.toml")).unwrap();
    record.push_str(&format!("\"{path}\" = \"managed\"\n"));
    fs::write(dir.join(".rootstock.toml"), record).unwrap();
}

/// Asserts that a refused update left README and the record at v1.0.0.
fn assert_untouched(dir: &Path) {
    assert_eq!(
        fs::read_to_string(dir.join("README.md")).unwrap(),
        README_V1
    );
    let record = fs::read_to_string(dir.join(".rootstock.toml")).unwrap();
    assert!(record.contains("version = \"v1.0.0\""), "{record}");
}

#[test]
fn leftover_markers_ignore_starter() {
    let repo = template_v1();
    let (_tmp, dir) = new_project(&repo);
    // A starter file with markers is the project's own business.
    fs::write(dir.join("NOTES.md"), LEFTOVER).unwrap();
    commit_project(&dir, "notes with markers");
    repo.file("tpl/README.md", README_V2).tag("v1.1.0");
    update(&dir).assert().code(0);
    assert_eq!(
        fs::read_to_string(dir.join("README.md")).unwrap(),
        README_V2
    );
}

#[cfg(unix)]
#[test]
fn leftover_markers_symlinked_parent_exits_1() {
    let repo = template_v1();
    let (tmp, dir) = new_project(&repo);
    // A managed path whose parent the project replaced with a symlink is never read.
    let elsewhere = tmp.path().join("elsewhere");
    fs::create_dir(&elsewhere).unwrap();
    fs::write(elsewhere.join("x.txt"), LEFTOVER).unwrap();
    std::os::unix::fs::symlink(&elsewhere, dir.join("ghost")).unwrap();
    add_managed_entry(&dir, "ghost/x.txt");
    commit_project(&dir, "odd project");
    repo.file("tpl/README.md", README_V2).tag("v1.1.0");
    update(&dir)
        .assert()
        .code(1)
        .stderr(predicate::str::contains(
            "`ghost/x.txt`: path goes through symlink",
        ))
        .stderr(predicate::str::contains("unresolved conflict markers").not());
    assert_untouched(&dir);
}

#[test]
fn leftover_markers_unsafe_inventory_path_exits_1() {
    let repo = template_v1();
    let (_tmp, dir) = new_project(&repo);
    add_managed_entry(&dir, "../outside.txt");
    commit_project(&dir, "odd record");
    repo.file("tpl/README.md", README_V2).tag("v1.1.0");
    update(&dir)
        .assert()
        .code(1)
        .stderr(predicate::str::contains(
            "record lists an unsafe path `../outside.txt`",
        ));
    assert_untouched(&dir);
}

const SUB_CONFIG: &str = "schema = 1\nroot = \"files\"\n";

/// A template that lives in `tpl/` of its repository; `docs/` holds no template.
fn subdir_template() -> TemplateRepo {
    let repo = TemplateRepo::new();
    repo.file("tpl/rootstock.toml", SUB_CONFIG)
        .file("tpl/files/a.txt", "one\n")
        .file("docs/readme.txt", "docs\n")
        .tag("v1.0.0");
    repo
}

fn file_url(repo: &TemplateRepo) -> String {
    format!("file://{}", repo.path().display())
}

fn new_with_subdir(repo: &TemplateRepo, dir: &Path) -> assert_cmd::assert::Assert {
    rootstock()
        .args(["new", &file_url(repo)])
        .arg(dir)
        .args(["--subdir", "tpl", "--defaults"])
        .assert()
}

#[test]
fn new_subdir_records_it_and_update_follows() {
    let repo = subdir_template();
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path().join("proj");
    new_with_subdir(&repo, &dir).code(0);
    assert_eq!(fs::read_to_string(dir.join("a.txt")).unwrap(), "one\n");
    let record = fs::read_to_string(dir.join(".rootstock.toml")).unwrap();
    assert!(record.contains("subdir = \"tpl\""), "{record}");
    commit_project(&dir, "init");

    repo.file("tpl/files/a.txt", "two\n").tag("v1.1.0");
    update(&dir).arg("--defaults").assert().code(0);
    assert_eq!(fs::read_to_string(dir.join("a.txt")).unwrap(), "two\n");
    let record = fs::read_to_string(dir.join(".rootstock.toml")).unwrap();
    assert!(record.contains("subdir = \"tpl\""), "{record}");
    assert!(record.contains("v1.1.0"), "{record}");
}

#[test]
fn new_subdir_with_head_renders_work_tree() {
    let repo = subdir_template();
    repo.file("tpl/files/a.txt", "wip\n");
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path().join("proj");
    rootstock()
        .args(["new", &repo.url()])
        .arg(&dir)
        .args(["--subdir", "tpl", "--version", "HEAD", "--defaults"])
        .assert()
        .code(0);
    assert_eq!(fs::read_to_string(dir.join("a.txt")).unwrap(), "wip\n");
}

#[test]
fn new_subdir_escaping_source_exits_1_before_git() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path().join("proj");
    for bad in ["../x", "/abs"] {
        // The source does not exist: validation must fail before git is touched.
        rootstock()
            .args(["new", "file:///nonexistent/rootstock-repo"])
            .arg(&dir)
            .args(["--subdir", bad, "--defaults"])
            .assert()
            .code(1)
            .stderr(predicate::str::contains("invalid template subdirectory"));
        assert!(!dir.exists());
    }
}

#[test]
fn new_subdir_without_config_exits_1() {
    let repo = subdir_template();
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path().join("proj");
    rootstock()
        .args(["new", &file_url(&repo)])
        .arg(&dir)
        .args(["--subdir", "docs", "--defaults"])
        .assert()
        .code(1)
        .stderr(predicate::str::contains("template not found at `docs`"));
    assert!(!dir.exists());
}

#[test]
fn update_fails_when_subdir_vanishes_in_target() {
    let repo = subdir_template();
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path().join("proj");
    new_with_subdir(&repo, &dir).code(0);
    commit_project(&dir, "init");
    repo.remove("tpl/rootstock.toml").tag("v1.1.0");

    let before_file = fs::read(dir.join("a.txt")).unwrap();
    let before_record = fs::read(dir.join(".rootstock.toml")).unwrap();
    update(&dir)
        .arg("--defaults")
        .assert()
        .code(1)
        .stderr(predicate::str::contains("template not found at `tpl`"));
    assert_eq!(fs::read(dir.join("a.txt")).unwrap(), before_file);
    assert_eq!(
        fs::read(dir.join(".rootstock.toml")).unwrap(),
        before_record
    );
}

#[test]
fn status_works_with_subdir() {
    let repo = subdir_template();
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path().join("proj");
    new_with_subdir(&repo, &dir).code(0);
    rootstock()
        .current_dir(&dir)
        .arg("status")
        .assert()
        .code(0)
        .stdout(predicate::str::contains("v1.0.0"));
}

#[test]
fn update_rejects_hand_edited_bad_subdir() {
    let repo = subdir_template();
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path().join("proj");
    new_with_subdir(&repo, &dir).code(0);
    let path = dir.join(".rootstock.toml");
    let text = fs::read_to_string(&path).unwrap();
    fs::write(&path, text.replace("subdir = \"tpl\"", "subdir = \"../x\"")).unwrap();
    commit_project(&dir, "init");
    for cmd in ["update", "status"] {
        rootstock()
            .current_dir(&dir)
            .arg(cmd)
            .assert()
            .code(1)
            .stderr(predicate::str::contains("invalid template subdirectory"));
    }
}

#[test]
fn new_subdir_without_version_uses_latest_tag() {
    let repo = subdir_template();
    repo.file("tpl/files/a.txt", "newer\n").tag("v1.1.0");
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path().join("proj");
    new_with_subdir(&repo, &dir).code(0);
    assert_eq!(fs::read_to_string(dir.join("a.txt")).unwrap(), "newer\n");
    let record = fs::read_to_string(dir.join(".rootstock.toml")).unwrap();
    assert!(record.contains("version = \"v1.1.0\""), "{record}");
}

#[test]
fn new_bad_subdir_leaves_existing_dir_untouched() {
    let repo = subdir_template();
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path().join("proj");
    fs::create_dir(&dir).unwrap();
    fs::write(dir.join("keep.txt"), b"mine\r\n").unwrap();
    rootstock()
        .args(["new", &file_url(&repo)])
        .arg(&dir)
        .args(["--subdir", "docs", "--defaults"])
        .assert()
        .code(1);
    assert_eq!(fs::read(dir.join("keep.txt")).unwrap(), b"mine\r\n");
    assert!(!dir.join(".rootstock.toml").exists());
    assert_eq!(fs::read_dir(&dir).unwrap().count(), 1);
}

#[test]
fn new_rejects_removed_include_root_key() {
    let repo = TemplateRepo::new();
    repo.file("rootstock.toml", "schema = 1\ninclude_root = \".\"\n")
        .file("a.txt", "a\n")
        .tag("v1.0.0");
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path().join("proj");
    rootstock()
        .args(["new", &repo.url()])
        .arg(&dir)
        .arg("--defaults")
        .assert()
        .code(1)
        .stderr(predicate::str::contains("include_root"));
    assert!(!dir.exists());
}

#[test]
fn new_from_a_template_that_is_itself_a_project() {
    let repo = TemplateRepo::new();
    repo.file("rootstock.toml", "schema = 1\n")
        .file(
            ".rootstock.toml",
            "schema = 1\nsource = \"/elsewhere\"\nversion = \"v9.9.9\"\ncommit = \"abc\"\n",
        )
        .file("README.md", "readme\n")
        .tag("v1.0.0");
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path().join("proj");
    rootstock()
        .args(["new", &repo.url()])
        .arg(&dir)
        .arg("--defaults")
        .assert()
        .code(0);
    assert_eq!(
        fs::read_to_string(dir.join("README.md")).unwrap(),
        "readme\n"
    );
    let record = fs::read_to_string(dir.join(".rootstock.toml")).unwrap();
    assert!(record.contains("version = \"v1.0.0\""), "{record}");
    assert!(
        !record.contains("v9.9.9") && !record.contains("/elsewhere"),
        "{record}"
    );
}

#[test]
fn new_runs_a_posix_hook_on_every_os() {
    let repo = TemplateRepo::new();
    // Shell built-ins and a redirection only: nothing but a POSIX `sh` is needed.
    let hook = "if command -v true >/dev/null 2>&1; then echo ran > hooked.txt; fi";
    repo.file(
        "rootstock.toml",
        &format!("schema = 1\n\n[[hooks]]\nrun = {hook:?}\nwhen = [\"new\"]\n"),
    )
    .tag("v1.0.0");
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path().join("proj");
    rootstock()
        .args(["new", &repo.url()])
        .arg(&dir)
        .args(["--defaults", "--trust"])
        .assert()
        .code(0);
    assert_eq!(
        fs::read_to_string(dir.join("hooked.txt")).unwrap().trim(),
        "ran"
    );
}

/// A global git config with the given body, and the temp dir that holds it.
fn git_global(body: &str) -> (TempDir, std::path::PathBuf) {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("gitconfig");
    fs::write(&path, body).unwrap();
    (tmp, path)
}

#[test]
fn new_keeps_lf_when_git_converts_line_endings() {
    let repo = TemplateRepo::new();
    repo.file("rootstock.toml", "schema = 1\n")
        .file("lf.txt", "one\ntwo\n")
        .bytes("crlf.txt", b"one\r\ntwo\r\n")
        .tag("v1.0.0");
    let (_cfg, global) = git_global("[core]\n\tautocrlf = true\n");
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path().join("proj");
    rootstock()
        .env("GIT_CONFIG_GLOBAL", &global)
        .args(["new", &repo.url()])
        .arg(&dir)
        .arg("--defaults")
        .assert()
        .code(0);
    assert_eq!(fs::read(dir.join("lf.txt")).unwrap(), b"one\ntwo\n");
    assert_eq!(fs::read(dir.join("crlf.txt")).unwrap(), b"one\r\ntwo\r\n");
}

#[test]
fn new_keeps_lf_for_text_auto_attributes_when_eol_is_crlf() {
    let repo = TemplateRepo::new();
    repo.file("rootstock.toml", "schema = 1\n")
        .file(".gitattributes", "* text=auto\n")
        .file("lf.txt", "one\ntwo\n")
        .tag("v1.0.0");
    let (_cfg, global) = git_global("[core]\n\teol = crlf\n");
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path().join("proj");
    rootstock()
        .env("GIT_CONFIG_GLOBAL", &global)
        .args(["new", &repo.url()])
        .arg(&dir)
        .arg("--defaults")
        .assert()
        .code(0);
    assert_eq!(fs::read(dir.join("lf.txt")).unwrap(), b"one\ntwo\n");
}

#[test]
fn new_emits_a_link_the_checkout_wrote_as_a_file() {
    let repo = TemplateRepo::new();
    repo.file("rootstock.toml", "schema = 1\n")
        .file("real.txt", "content\n");
    repo.tree_symlink("link", "real.txt").tag_staged("v1.0.0");
    // Git told not to make links writes the target into a plain file.
    let (_cfg, global) = git_global("[core]\n\tsymlinks = false\n");
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path().join("proj");
    rootstock()
        .env("GIT_CONFIG_GLOBAL", &global)
        .args(["new", &repo.url()])
        .arg(&dir)
        .arg("--defaults")
        .assert()
        .code(0);
    assert_eq!(
        fs::read_link(dir.join("link")).unwrap().to_str().unwrap(),
        "real.txt"
    );
}

#[test]
fn new_emits_the_executable_bit_of_the_tree() {
    let repo = TemplateRepo::new();
    repo.file("rootstock.toml", "schema = 1\n")
        .file("run.sh", "#!/bin/sh\n")
        .file("plain.txt", "x\n");
    repo.exec("run.sh").tag("v1.0.0");
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path().join("proj");
    rootstock()
        .args(["new", &repo.url()])
        .arg(&dir)
        .arg("--defaults")
        .arg("--git")
        .assert()
        .code(0);
    // Windows keeps the bit only in the index of the project.
    #[cfg(not(unix))]
    {
        let index = git_in(&dir, &["ls-files", "-s"]);
        assert!(
            index
                .lines()
                .any(|l| l.starts_with("100755") && l.ends_with("run.sh")),
            "{index}"
        );
    }
    // Elsewhere the disk has it.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode_of = |name: &str| fs::metadata(dir.join(name)).unwrap().permissions().mode();
        assert_eq!(mode_of("run.sh") & 0o111, 0o111);
        assert_eq!(mode_of("plain.txt") & 0o111, 0);
    }
}
