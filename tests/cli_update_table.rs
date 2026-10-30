//! End-to-end checks of the update table: every scenario drives the binary
//! through `new` and `update`.

mod support;

use std::fs;
use std::path::{Path, PathBuf};

use assert_cmd::Command;
use support::{TemplateRepo, commit_project};
use tempfile::TempDir;

/// Template config with the given starter and exclude rules.
fn config(starter: &[&str], exclude: &[&str]) -> String {
    let list = |items: &[&str]| {
        items
            .iter()
            .map(|s| format!("{s:?}"))
            .collect::<Vec<_>>()
            .join(", ")
    };
    format!(
        "schema = 1\nroot = \"tpl\"\n\n[files]\nstarter = [{}]\nexclude = [{}]\n",
        list(starter),
        list(exclude)
    )
}

/// A template repository whose v1.0.0 holds `files` under `tpl/`.
fn template(cfg: &str, files: &[(&str, &str)]) -> TemplateRepo {
    let repo = TemplateRepo::new();
    repo.file("rootstock.toml", cfg);
    for (path, content) in files {
        repo.file(&format!("tpl/{path}"), content);
    }
    repo.tag("v1.0.0");
    repo
}

fn rootstock() -> Command {
    let mut cmd = Command::cargo_bin("rootstock").unwrap();
    cmd.env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1");
    cmd
}

/// Runs `rootstock new --defaults` at v1.0.0 and commits the project.
fn new_project(repo: &TemplateRepo) -> (TempDir, PathBuf) {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path().join("proj");
    rootstock()
        .args(["new", &repo.url()])
        .arg(&dir)
        .args(["--defaults", "--version", "v1.0.0"])
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

fn read(dir: &Path, path: &str) -> String {
    fs::read_to_string(dir.join(path)).unwrap()
}

fn record(dir: &Path) -> String {
    read(dir, ".rootstock.toml")
}

/// Names of the entries directly under `dir`, as stored by the filesystem.
fn names(dir: &Path) -> Vec<String> {
    let mut out: Vec<String> = fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n != ".git")
        .collect();
    out.sort();
    out
}

const EIGHT: &str = "l1\nl2\nl3\nl4\nl5\nl6\nl7\nl8\n";
const EIGHT_FIRST: &str = "L1-project\nl2\nl3\nl4\nl5\nl6\nl7\nl8\n";
const EIGHT_LAST: &str = "l1\nl2\nl3\nl4\nl5\nl6\nl7\nL8-template\n";
const EIGHT_BOTH: &str = "L1-project\nl2\nl3\nl4\nl5\nl6\nl7\nL8-template\n";

#[test]
fn e2e_new_starter_file_created() {
    let repo = template(
        &config(&["NOTES.md"], &[]),
        &[("NOTES.md", "notes\n"), ("a.txt", "a\n")],
    );
    let (_tmp, dir) = new_project(&repo);
    assert_eq!(read(&dir, "NOTES.md"), "notes\n");
    assert!(record(&dir).contains("\"NOTES.md\" = \"starter\""));
}

#[test]
fn e2e_added_by_both() {
    let repo = template(&config(&[], &[]), &[("a.txt", "a\n")]);
    let (_tmp, dir) = new_project(&repo);
    fs::write(dir.join("X.txt"), "mine\n").unwrap();
    commit_project(&dir, "add X");
    repo.file("tpl/X.txt", "theirs\n").tag("v1.1.0");
    update(&dir)
        .assert()
        .code(2)
        .stdout("added-by-both X.txt\n");
    let text = read(&dir, "X.txt");
    assert!(text.contains("<<<<<<< project\nmine\n"), "{text}");
    assert!(text.contains("theirs\n>>>>>>> template v1.1.0"), "{text}");
}

#[test]
fn e2e_deleted_starter_not_recreated_after_gap() {
    let repo = template(
        &config(&["NOTES.md"], &[]),
        &[("NOTES.md", "notes\n"), ("a.txt", "a\n")],
    );
    let (_tmp, dir) = new_project(&repo);
    fs::remove_file(dir.join("NOTES.md")).unwrap();
    commit_project(&dir, "drop notes");

    // v2 drops the starter, v3 brings it back.
    repo.remove("tpl/NOTES.md")
        .file("tpl/a.txt", "a2\n")
        .tag("v2.0.0");
    repo.file("tpl/NOTES.md", "notes v3\n").tag("v3.0.0");

    update(&dir)
        .args(["--to", "v2.0.0"])
        .assert()
        .code(0)
        .stdout("");
    assert!(!dir.join("NOTES.md").exists());
    assert!(record(&dir).contains("\"NOTES.md\" = \"starter\""));
    commit_project(&dir, "v2");

    update(&dir)
        .args(["--to", "v3.0.0"])
        .assert()
        .code(0)
        .stdout("");
    assert!(!dir.join("NOTES.md").exists());
    assert_eq!(read(&dir, "a.txt"), "a2\n");
    assert!(record(&dir).contains("version = \"v3.0.0\""));
}

#[test]
fn e2e_starter_present_untouched() {
    let repo = template(
        &config(&["NOTES.md"], &[]),
        &[("NOTES.md", "notes\n"), ("a.txt", "a\n")],
    );
    let (_tmp, dir) = new_project(&repo);
    fs::write(dir.join("NOTES.md"), "mine\n").unwrap();
    commit_project(&dir, "edit notes");
    repo.file("tpl/NOTES.md", "notes v2\n")
        .file("tpl/a.txt", "a2\n")
        .tag("v1.1.0");
    update(&dir).assert().code(0).stdout("");
    assert_eq!(read(&dir, "NOTES.md"), "mine\n");
    assert_eq!(read(&dir, "a.txt"), "a2\n");
}

#[test]
fn e2e_deleted_by_project_changed_by_template() {
    let repo = template(&config(&[], &[]), &[("a.txt", "a\n"), ("b.txt", "b\n")]);
    let (_tmp, dir) = new_project(&repo);
    fs::remove_file(dir.join("a.txt")).unwrap();
    commit_project(&dir, "rm a");
    repo.file("tpl/a.txt", "a2\n").tag("v1.1.0");
    update(&dir)
        .assert()
        .code(2)
        .stdout("deleted-by-project a.txt\n");
    assert!(!dir.join("a.txt").exists());
}

#[test]
fn e2e_removed_by_template_edited() {
    let repo = template(&config(&[], &[]), &[("a.txt", "a\n"), ("b.txt", "b\n")]);
    let (_tmp, dir) = new_project(&repo);
    fs::write(dir.join("a.txt"), "mine\n").unwrap();
    commit_project(&dir, "edit a");
    repo.remove("tpl/a.txt").tag("v1.1.0");
    update(&dir)
        .assert()
        .code(2)
        .stdout("removed-by-template a.txt\n");
    assert_eq!(read(&dir, "a.txt"), "mine\n");
}

#[test]
fn e2e_removed_by_template_already_absent() {
    let repo = template(&config(&[], &[]), &[("a.txt", "a\n"), ("b.txt", "b\n")]);
    let (_tmp, dir) = new_project(&repo);
    fs::remove_file(dir.join("a.txt")).unwrap();
    commit_project(&dir, "rm a");
    repo.remove("tpl/a.txt").tag("v1.1.0");
    update(&dir).assert().code(0).stdout("");
    assert!(!dir.join("a.txt").exists());
    assert!(!record(&dir).contains("a.txt"));
}

#[test]
fn e2e_managed_to_starter() {
    let repo = template(&config(&[], &[]), &[("a.txt", "a\n")]);
    let (_tmp, dir) = new_project(&repo);
    repo.file("rootstock.toml", &config(&["a.txt"], &[]))
        .file("tpl/a.txt", "a2\n")
        .tag("v1.1.0");
    update(&dir).assert().code(0).stdout("");
    assert_eq!(read(&dir, "a.txt"), "a\n");
    assert!(record(&dir).contains("\"a.txt\" = \"starter\""));
}

#[test]
fn e2e_starter_to_managed() {
    let repo = template(&config(&["a.txt"], &[]), &[("a.txt", EIGHT)]);
    let (_tmp, dir) = new_project(&repo);
    fs::write(dir.join("a.txt"), EIGHT_FIRST).unwrap();
    commit_project(&dir, "edit a");
    repo.file("rootstock.toml", &config(&[], &[]))
        .file("tpl/a.txt", EIGHT_LAST)
        .tag("v1.1.0");
    update(&dir).assert().code(0).stdout("");
    assert_eq!(read(&dir, "a.txt"), EIGHT_BOTH);
    assert!(record(&dir).contains("\"a.txt\" = \"managed\""));
}

#[test]
fn e2e_excluded_now_removed() {
    let repo = template(&config(&[], &[]), &[("a.txt", "a\n"), ("b.txt", "b\n")]);
    let (_tmp, dir) = new_project(&repo);
    repo.file("rootstock.toml", &config(&[], &["a.txt"]))
        .tag("v1.1.0");
    update(&dir).assert().code(0).stdout("");
    assert!(!dir.join("a.txt").exists());
    assert!(dir.join("b.txt").exists());
    assert!(!record(&dir).contains("a.txt"));
}

#[test]
fn e2e_binary_side_file() {
    let repo = TemplateRepo::new();
    repo.file("rootstock.toml", &config(&[], &[]))
        .bytes("tpl/img.bin", &[0, 1, 0])
        .tag("v1.0.0");
    let (_tmp, dir) = new_project(&repo);
    fs::write(dir.join("img.bin"), [0, 3, 0]).unwrap();
    commit_project(&dir, "edit img");
    repo.bytes("tpl/img.bin", &[0, 2, 0]).tag("v1.1.0");
    update(&dir).assert().code(2).stdout("binary img.bin\n");
    assert_eq!(fs::read(dir.join("img.bin")).unwrap(), [0, 3, 0]);
    assert_eq!(
        fs::read(dir.join("img.bin.rootstock-new")).unwrap(),
        [0, 2, 0]
    );
}

#[test]
fn e2e_binary_unedited_replaced() {
    let repo = TemplateRepo::new();
    repo.file("rootstock.toml", &config(&[], &[]))
        .bytes("tpl/img.bin", &[0, 1, 0])
        .tag("v1.0.0");
    let (_tmp, dir) = new_project(&repo);
    repo.bytes("tpl/img.bin", &[0, 2, 0]).tag("v1.1.0");
    update(&dir).assert().code(0).stdout("");
    assert_eq!(fs::read(dir.join("img.bin")).unwrap(), [0, 2, 0]);
    assert!(!dir.join("img.bin.rootstock-new").exists());
}

#[cfg(unix)]
#[test]
fn e2e_exec_bit_follows_template() {
    use std::os::unix::fs::PermissionsExt;
    let repo = template(&config(&[], &[]), &[("run.sh", "echo hi\n")]);
    let (_tmp, dir) = new_project(&repo);
    let mode = |dir: &Path| {
        fs::metadata(dir.join("run.sh"))
            .unwrap()
            .permissions()
            .mode()
    };
    assert_eq!(mode(&dir) & 0o111, 0);
    repo.exec("tpl/run.sh").tag("v1.1.0");
    update(&dir).assert().code(0).stdout("");
    assert_eq!(mode(&dir) & 0o111, 0o111);
    assert_eq!(read(&dir, "run.sh"), "echo hi\n");
}

#[cfg(unix)]
#[test]
fn e2e_symlink_retargeted() {
    let repo = template(&config(&[], &[]), &[("a.txt", "a\n"), ("b.txt", "b\n")]);
    repo.symlink("tpl/link", "a.txt").tag("v1.0.1");
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path().join("proj");
    rootstock()
        .args(["new", &repo.url()])
        .arg(&dir)
        .args(["--defaults", "--version", "v1.0.1"])
        .assert()
        .code(0);
    commit_project(&dir, "init");
    repo.symlink("tpl/link", "b.txt").tag("v1.1.0");
    update(&dir).assert().code(0).stdout("");
    assert_eq!(fs::read_link(dir.join("link")).unwrap(), Path::new("b.txt"));
}

#[cfg(unix)]
#[test]
fn e2e_symlink_diverged() {
    let repo = template(&config(&[], &[]), &[("a.txt", "a\n"), ("b.txt", "b\n")]);
    repo.symlink("tpl/link", "a.txt").tag("v1.0.1");
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path().join("proj");
    rootstock()
        .args(["new", &repo.url()])
        .arg(&dir)
        .args(["--defaults", "--version", "v1.0.1"])
        .assert()
        .code(0);
    fs::remove_file(dir.join("link")).unwrap();
    std::os::unix::fs::symlink("mine.txt", dir.join("link")).unwrap();
    commit_project(&dir, "retarget");
    repo.symlink("tpl/link", "b.txt").tag("v1.1.0");
    update(&dir).assert().code(2).stdout("symlink link\n");
    assert_eq!(
        fs::read_link(dir.join("link")).unwrap(),
        Path::new("mine.txt")
    );
}

#[test]
fn e2e_crlf_merge_preserves_endings() {
    let crlf = |s: &str| s.replace('\n', "\r\n");
    let repo = template(&config(&[], &[]), &[("a.txt", &crlf(EIGHT))]);
    let (_tmp, dir) = new_project(&repo);
    fs::write(dir.join("a.txt"), crlf(EIGHT_FIRST)).unwrap();
    commit_project(&dir, "edit a");
    repo.file("tpl/a.txt", &crlf(EIGHT_LAST)).tag("v1.1.0");
    update(&dir).assert().code(0).stdout("");
    assert_eq!(
        fs::read(dir.join("a.txt")).unwrap(),
        crlf(EIGHT_BOTH).into_bytes()
    );
}

#[test]
fn e2e_no_final_newline_preserved() {
    let trim = |s: &str| s.trim_end_matches('\n').to_string();
    let repo = template(&config(&[], &[]), &[("a.txt", &trim(EIGHT))]);
    let (_tmp, dir) = new_project(&repo);
    fs::write(dir.join("a.txt"), trim(EIGHT_FIRST)).unwrap();
    commit_project(&dir, "edit a");
    repo.file("tpl/a.txt", &trim(EIGHT_LAST)).tag("v1.1.0");
    update(&dir).assert().code(0).stdout("");
    assert_eq!(read(&dir, "a.txt"), trim(EIGHT_BOTH));
}

#[test]
fn e2e_when_false_not_recorded_then_enabled() {
    let cfg = format!(
        "{}\n[[questions]]\nname = \"linear\"\ntype = \"bool\"\ndefault = false\n\n\
         [[questions]]\nname = \"prefix\"\ntype = \"str\"\nwhen = \"linear\"\ndefault = \"DEN\"\n",
        config(&[], &[])
    );
    let repo = template(
        &cfg,
        &[
            (
                "info.txt.jinja",
                "{% if linear %}on{% else %}off{% endif %}\n",
            ),
            (
                "{% if linear %}prefix.txt{% endif %}.jinja",
                "{{ prefix }}\n",
            ),
        ],
    );
    let (_tmp, dir) = new_project(&repo);
    assert!(!record(&dir).contains("prefix"), "{}", record(&dir));
    assert!(!dir.join("prefix.txt").exists());
    assert_eq!(read(&dir, "info.txt"), "off\n");
    repo.file(
        "tpl/info.txt.jinja",
        "linear: {% if linear %}on{% else %}off{% endif %}\n",
    )
    .tag("v1.1.0");
    update(&dir)
        .args(["--data", "linear=true", "--data", "prefix=ABC"])
        .assert()
        .code(0)
        .stdout("");
    assert_eq!(read(&dir, "prefix.txt"), "ABC\n");
    assert_eq!(read(&dir, "info.txt"), "linear: on\n");
    let rec = record(&dir);
    assert!(rec.contains("prefix = \"ABC\""), "{rec}");
    assert!(rec.contains("linear = true"), "{rec}");
}

#[test]
fn e2e_case_only_rename() {
    let repo = template(&config(&[], &[]), &[("Readme.md", EIGHT)]);
    let (_tmp, edited) = new_project(&repo);
    let (_tmp2, plain) = new_project(&repo);
    fs::write(edited.join("Readme.md"), EIGHT_FIRST).unwrap();
    commit_project(&edited, "edit readme");
    repo.git(&["mv", "tpl/Readme.md", "tpl/README.md"]);
    repo.file("tpl/README.md", EIGHT_LAST).tag("v1.1.0");

    // Either way the unedited project ends with the new name only.
    update(&plain).assert().code(0).stdout("");
    assert_eq!(names(&plain), [".rootstock.toml", "README.md"]);
    assert_eq!(read(&plain, "README.md"), EIGHT_LAST);

    if case_insensitive(edited.parent().unwrap()) {
        // Both names are one file: the rename carries the project's edit along.
        update(&edited).assert().code(0).stdout("");
        assert_eq!(names(&edited), [".rootstock.toml", "README.md"]);
        assert_eq!(read(&edited, "README.md"), EIGHT_BOTH);
        let rec = record(&edited);
        assert!(rec.contains("\"README.md\" = \"managed\""), "{rec}");
        assert!(!rec.contains("Readme.md"), "{rec}");
    } else {
        // Two names, two files: a delete of an edited file plus an add.
        update(&edited)
            .assert()
            .code(2)
            .stdout("removed-by-template Readme.md\n");
        assert_eq!(
            names(&edited),
            [".rootstock.toml", "README.md", "Readme.md"]
        );
        assert_eq!(read(&edited, "Readme.md"), EIGHT_FIRST);
        assert_eq!(read(&edited, "README.md"), EIGHT_LAST);
        let rec = record(&edited);
        assert!(rec.contains("\"README.md\" = \"managed\""), "{rec}");
    }
}

/// True when the filesystem under `dir` finds a file under another case of its name.
fn case_insensitive(dir: &Path) -> bool {
    let probe = TempDir::new_in(dir).unwrap();
    fs::write(probe.path().join("case-probe"), "").unwrap();
    probe.path().join("CASE-PROBE").exists()
}

#[test]
fn e2e_file_becomes_dir() {
    let repo = template(&config(&[], &[]), &[("docs", "d\n"), ("a.txt", "a\n")]);
    let (_tmp, plain) = new_project(&repo);
    let (_tmp2, edited) = new_project(&repo);
    fs::write(edited.join("docs"), "mine\n").unwrap();
    commit_project(&edited, "edit docs");
    repo.remove("tpl/docs")
        .file("tpl/docs/x.txt", "x\n")
        .tag("v1.1.0");

    update(&plain).assert().code(0).stdout("");
    assert_eq!(read(&plain, "docs/x.txt"), "x\n");

    update(&edited)
        .assert()
        .code(2)
        .stdout("removed-by-template docs\nblocked docs/x.txt\n");
    assert_eq!(read(&edited, "docs"), "mine\n");
    assert!(!record(&edited).contains("docs/x.txt"));
}

#[test]
fn e2e_dir_becomes_file() {
    let repo = template(
        &config(&[], &[]),
        &[("docs/x.txt", "x\n"), ("a.txt", "a\n")],
    );
    let (_tmp, plain) = new_project(&repo);
    let (_tmp2, kept) = new_project(&repo);
    fs::write(kept.join("docs/mine.txt"), "mine\n").unwrap();
    commit_project(&kept, "add mine");
    repo.remove("tpl/docs/x.txt");
    fs::remove_dir(repo.path().join("tpl/docs")).unwrap();
    repo.file("tpl/docs", "d\n").tag("v1.1.0");

    update(&plain).assert().code(0).stdout("");
    assert_eq!(read(&plain, "docs"), "d\n");

    update(&kept).assert().code(2).stdout("blocked docs\n");
    assert_eq!(read(&kept, "docs/mine.txt"), "mine\n");
    assert!(!kept.join("docs/x.txt").exists());
    assert!(!record(&kept).contains("\"docs\""));
}

#[test]
fn e2e_dir_becomes_file_empty_subdir_blocks() {
    let repo = template(
        &config(&[], &[]),
        &[("docs/x.txt", "x\n"), ("a.txt", "a\n")],
    );
    let (_tmp, dir) = new_project(&repo);
    // git does not track empty directories, so the tree stays clean.
    fs::create_dir(dir.join("docs/empty")).unwrap();
    repo.remove("tpl/docs/x.txt");
    fs::remove_dir(repo.path().join("tpl/docs")).unwrap();
    repo.file("tpl/docs", "d\n").tag("v1.1.0");

    update(&dir).assert().code(2).stdout("blocked docs\n");
    assert!(dir.join("docs/empty").is_dir());
    assert!(!dir.join("docs/x.txt").exists());
    let rec = record(&dir);
    assert!(rec.contains("version = \"v1.1.0\""), "{rec}");
    assert!(!rec.contains("\"docs\""), "{rec}");
    assert!(!rec.contains("docs/x.txt"), "{rec}");
}
