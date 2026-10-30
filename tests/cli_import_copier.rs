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

const HARNESS_V033: &str = include_str!("fixtures/copier/harness-v0.3.3.yml");

const HARNESS_ANSWERS_BODY: &str = "# Changes here will be overwritten by Copier; NEVER EDIT MANUALLY\n{{ _copier_answers|to_nice_yaml -}}\n";

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
fn converts_the_harness_templates() {
    for yaml in [HARNESS, HARNESS_V033] {
        let repo = TemplateRepo::new();
        repo.file("copier.yml", yaml)
            .file(ANSWERS_FILE, HARNESS_ANSWERS_BODY);
        import(repo.path())
            .code(0)
            .stdout(predicate::str::is_empty());
        assert!(repo.path().join("rootstock.toml").exists());
        assert!(!repo.path().join(ANSWERS_FILE).exists());
    }
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
    fs::create_dir_all(repo.path().join("template")).unwrap();
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

#[test]
fn symlink_under_the_root_is_a_failure() {
    let repo = small_template();
    #[cfg(unix)]
    repo.symlink("template/link.txt", "hello.txt.jinja");
    import(repo.path())
        .code(1)
        .stderr(predicate::str::contains("template/link.txt"));
    assert!(!repo.path().join("rootstock.toml").exists());
}

#[test]
fn update_only_exclude_without_a_pair_is_a_failure() {
    let repo = TemplateRepo::new();
    repo.file(
        "copier.yml",
        "_exclude:\n  - \"{% if _copier_operation == 'update' %}/a.txt{% endif %}\"\n",
    );
    import(repo.path())
        .code(1)
        .stderr(predicate::str::contains("matching `_skip_if_exists`"));
    assert!(!repo.path().join("rootstock.toml").exists());
}

#[test]
fn existing_config_leaves_copier_yml_untouched() {
    let repo = small_template();
    repo.file("rootstock.toml", "schema = 1\n");
    import(repo.path()).code(1);
    assert_eq!(
        fs::read(repo.path().join("copier.yml")).unwrap(),
        SMALL.as_bytes()
    );
}

#[cfg(unix)]
#[test]
fn bad_roots_do_not_touch_files_outside_the_template() {
    use std::os::unix::fs::symlink;
    let tmp = TempDir::new().unwrap();
    let outside = tmp.path().join("outside");
    fs::create_dir_all(&outside).unwrap();
    let victim = outside.join("{{ _copier_conf.answers_file }}.jinja");
    fs::write(&victim, "x").unwrap();
    let repo = tmp.path().join("repo");
    fs::create_dir_all(&repo).unwrap();

    fs::write(repo.join("copier.yml"), "_subdirectory: ../outside\n").unwrap();
    import(&repo).code(1).stderr(predicate::str::contains(
        "relative path inside the template",
    ));
    assert!(victim.exists());

    fs::write(repo.join("copier.yml"), "_subdirectory: tpl\n").unwrap();
    symlink(&outside, repo.join("tpl")).unwrap();
    import(&repo)
        .code(1)
        .stderr(predicate::str::contains("symbolic link"));
    assert!(victim.exists());
    assert!(!repo.join("rootstock.toml").exists());

    fs::remove_file(repo.join("tpl")).unwrap();
    import(&repo)
        .code(1)
        .stderr(predicate::str::contains("does not exist"));
}

#[cfg(unix)]
#[test]
fn copier_yml_must_be_a_regular_file() {
    let tmp = TempDir::new().unwrap();
    fs::write(tmp.path().join("real.yml"), SMALL).unwrap();
    std::os::unix::fs::symlink("real.yml", tmp.path().join("copier.yml")).unwrap();
    import(tmp.path()).code(1);
    assert!(!tmp.path().join("rootstock.toml").exists());

    let tmp = TempDir::new().unwrap();
    fs::create_dir(tmp.path().join("copier.yml")).unwrap();
    import(tmp.path()).code(1);
}

#[cfg(unix)]
#[test]
fn rootstock_toml_is_world_readable() {
    use std::os::unix::fs::PermissionsExt;
    let repo = small_template();
    import(repo.path()).code(0);
    let mode = fs::metadata(repo.path().join("rootstock.toml"))
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(mode & 0o777, 0o644);
}

#[cfg(unix)]
#[test]
fn failed_removal_says_the_config_is_already_written() {
    use std::os::unix::fs::PermissionsExt;
    let repo = small_template();
    let template = repo.path().join("template");
    fs::set_permissions(&template, fs::Permissions::from_mode(0o555)).unwrap();
    let probe = template.join("probe");
    if fs::write(&probe, "x").is_ok() {
        // Running as root: permissions do not apply.
        let _ = fs::remove_file(&probe);
        fs::set_permissions(&template, fs::Permissions::from_mode(0o755)).unwrap();
        eprintln!("skipped: directory permissions are not enforced");
        return;
    }
    let assert = import(repo.path())
        .code(1)
        .stderr(predicate::str::contains("already written"));
    drop(assert);
    fs::set_permissions(&template, fs::Permissions::from_mode(0o755)).unwrap();
    assert!(repo.path().join("rootstock.toml").exists());
    assert!(repo.path().join(ANSWERS_FILE).exists());
}
