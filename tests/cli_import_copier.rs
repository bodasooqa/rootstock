mod support;

use std::fs;
use std::path::Path;

use assert_cmd::Command;
use predicates::prelude::*;
use support::TemplateRepo;
use tempfile::TempDir;

const ANSWERS_FILE: &str = "template/{{ _copier_conf.answers_file }}.jinja";
const ANSWERS_BODY: &str =
    "# Changes here will be overwritten by Copier\n{{ _copier_answers|to_nice_yaml -}}\n";

const SMALL: &str =
    "_subdirectory: template\n_templates_suffix: .jinja\nname:\n  type: str\n  default: demo\n";

const HARNESS: &str = include_str!("fixtures/copier/harness-v0.4.5.yml");

fn rootstock() -> Command {
    let mut cmd = Command::cargo_bin("rootstock").unwrap();
    cmd.env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1");
    cmd
}

fn import(dir: &Path) -> assert_cmd::assert::Assert {
    rootstock().arg("import-copier").current_dir(dir).assert()
}

fn small_template() -> TemplateRepo {
    let repo = TemplateRepo::new();
    repo.file("copier.yml", SMALL)
        .file(ANSWERS_FILE, ANSWERS_BODY)
        .file("template/hello.txt.jinja", "hello {{ name }}\n");
    repo
}

#[test]
fn converts_a_copier_template() {
    let repo = small_template();
    import(repo.path())
        .code(0)
        .stdout(predicate::str::is_empty())
        .stderr(predicate::str::contains("wrote rootstock.toml"))
        .stderr(predicate::str::contains("removed"));
    let config = fs::read_to_string(repo.path().join("rootstock.toml")).unwrap();
    assert!(config.contains("schema = 1"));
    assert!(config.contains("root = \"template\""));
    assert!(repo.path().join("copier.yml").exists());
    assert!(!repo.path().join(ANSWERS_FILE).exists());
    assert!(repo.path().join("template/hello.txt.jinja").exists());
}

#[test]
fn converts_the_harness_template() {
    let repo = TemplateRepo::new();
    repo.file("copier.yml", HARNESS);
    fs::create_dir_all(repo.path().join("template")).unwrap();
    import(repo.path())
        .code(0)
        .stdout(predicate::str::is_empty());
    assert!(repo.path().join("rootstock.toml").exists());
}

#[test]
fn new_works_on_the_converted_template() {
    let repo = small_template();
    import(repo.path()).code(0);
    repo.tag("v1.0.0");
    let tmp = TempDir::new().unwrap();
    let project = tmp.path().join("proj");
    rootstock()
        .args(["new", &repo.url()])
        .arg(&project)
        .arg("--defaults")
        .assert()
        .code(0);
    assert_eq!(
        fs::read_to_string(project.join("hello.txt")).unwrap(),
        "hello demo\n"
    );
    assert!(!project.join(".copier-answers.yml").exists());
}

#[test]
fn existing_rootstock_toml_is_an_error() {
    let repo = small_template();
    repo.file("rootstock.toml", "schema = 1\n");
    import(repo.path())
        .code(1)
        .stderr(predicate::str::contains("rootstock.toml"));
    assert_eq!(
        fs::read_to_string(repo.path().join("rootstock.toml")).unwrap(),
        "schema = 1\n"
    );
    assert!(repo.path().join(ANSWERS_FILE).exists());
}

#[test]
fn both_copier_files_are_an_error() {
    let repo = small_template();
    repo.file("copier.yaml", SMALL);
    import(repo.path()).code(1);
    assert!(!repo.path().join("rootstock.toml").exists());
    assert!(repo.path().join(ANSWERS_FILE).exists());
}

#[test]
fn copier_yaml_is_accepted() {
    let repo = TemplateRepo::new();
    repo.file("copier.yaml", SMALL);
    import(repo.path()).code(0);
    assert!(repo.path().join("rootstock.toml").exists());
}

#[test]
fn missing_copier_yml_is_an_error() {
    let dir = TempDir::new().unwrap();
    import(dir.path())
        .code(1)
        .stderr(predicate::str::contains("copier.yml"));
    assert!(!dir.path().join("rootstock.toml").exists());
}

#[test]
fn invalid_yaml_is_an_error() {
    let repo = TemplateRepo::new();
    repo.file("copier.yml", "a: [unclosed\n");
    import(repo.path()).code(1);
    assert!(!repo.path().join("rootstock.toml").exists());
}

#[test]
fn all_findings_are_listed_and_nothing_is_written() {
    let repo = TemplateRepo::new();
    repo.file(
        "copier.yml",
        "_subdirectory: template\n_jinja_extensions: [x]\nq:\n  type: int\n",
    )
    .file(ANSWERS_FILE, ANSWERS_BODY)
    .file("template/a.txt.jinja", "{{ _copier_conf.src_path }}");
    import(repo.path())
        .code(1)
        .stdout(predicate::str::is_empty())
        .stderr(predicate::str::contains(
            "copier.yml uses features rootstock cannot convert:",
        ))
        .stderr(predicate::str::contains("_jinja_extensions"))
        .stderr(predicate::str::contains("question `q`"))
        .stderr(predicate::str::contains("template/a.txt.jinja"));
    assert!(!repo.path().join("rootstock.toml").exists());
    assert!(repo.path().join(ANSWERS_FILE).exists());
}
