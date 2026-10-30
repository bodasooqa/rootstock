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
        let config = fs::read_to_string(repo.path().join("rootstock.toml")).unwrap();
        assert!(config.contains("\"/.copier-answers.yml\""), "{config}");
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

// ---- project side ----

use support::{commit_project, git_in};

const PROJECT_COPIER: &str = "_subdirectory: template\n_templates_suffix: .jinja\n_skip_if_exists:\n  - /start.txt\nname:\n  type: str\n  default: demo\nflavor:\n  type: str\n  default: plain\n";

/// A Copier-format template at v1.0.0: a managed file and a starter file.
fn copier_template() -> TemplateRepo {
    let repo = TemplateRepo::new();
    repo.file("copier.yml", PROJECT_COPIER)
        .file(ANSWERS_FILE, ANSWERS_BODY)
        .file("template/managed.txt.jinja", "managed {{ name }} v1\n")
        .file("template/start.txt.jinja", "start {{ flavor }}\n")
        .tag("v1.0.0");
    repo
}

/// A committed Copier project at `commit_name` answering `name = shop`.
fn copier_project(repo: &TemplateRepo, commit_line: &str) -> TempDir {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    fs::write(
        dir.join(".copier-answers.yml"),
        format!(
            "# Changes here will be overwritten by Copier\n_commit: {commit_line}\n_src_path: {}\nname: shop\n",
            repo.url()
        ),
    )
    .unwrap();
    fs::write(dir.join("managed.txt"), "managed shop v1\n").unwrap();
    fs::write(dir.join("notes.bin"), [0u8, 159, 146, 150, b'\r', b'\n']).unwrap();
    commit_project(dir, "copier project");
    tmp
}

fn snapshot(dir: &Path) -> Vec<(String, Vec<u8>)> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for item in fs::read_dir(&d).unwrap() {
            let path = item.unwrap().path();
            if path.file_name().is_some_and(|n| n == ".git") {
                continue;
            }
            if path.is_dir() {
                stack.push(path);
            } else {
                let rel = path
                    .strip_prefix(dir)
                    .unwrap()
                    .to_string_lossy()
                    .into_owned();
                out.push((rel, fs::read(&path).unwrap()));
            }
        }
    }
    out.sort();
    out
}

fn imports_nothing(dir: &Path, stderr: &str) {
    let before = snapshot(dir);
    import(dir)
        .code(1)
        .stdout(predicate::str::is_empty())
        .stderr(predicate::str::contains(stderr));
    assert_eq!(snapshot(dir), before);
}

#[test]
fn imports_a_copier_project() {
    let repo = copier_template();
    let project = copier_project(&repo, "v1.0.0");
    let dir = project.path();
    // The project deleted its starter file after generation: here it never existed.
    let managed = fs::read(dir.join("managed.txt")).unwrap();
    let bin = fs::read(dir.join("notes.bin")).unwrap();
    import(dir)
        .code(0)
        .stdout(predicate::str::is_empty())
        .stderr(predicate::str::contains("wrote .rootstock.toml"))
        .stderr(predicate::str::contains("removed .copier-answers.yml"));
    assert!(!dir.join(".copier-answers.yml").exists());
    assert_eq!(fs::read(dir.join("managed.txt")).unwrap(), managed);
    assert_eq!(fs::read(dir.join("notes.bin")).unwrap(), bin);
    assert!(!dir.join("start.txt").exists());

    let record = fs::read_to_string(dir.join(".rootstock.toml")).unwrap();
    assert!(record.contains("schema = 1"), "{record}");
    assert!(record.contains("version = \"v1.0.0\""), "{record}");
    assert!(
        record.contains(&format!("commit = \"{}\"", repo.commit("v1.0.0"))),
        "{record}"
    );
    let canonical = support::canon(repo.path());
    assert_eq!(
        support::recorded_source(&record),
        canonical.to_string_lossy(),
        "{record}"
    );
    assert!(record.contains("name = \"shop\""), "{record}");
    // Defaults of the questions without an answer are recorded too.
    assert!(record.contains("flavor = \"plain\""), "{record}");
    assert!(record.contains("\"managed.txt\" = \"managed\""), "{record}");
    assert!(record.contains("\"start.txt\" = \"starter\""), "{record}");
    assert!(!record.contains("copier-answers"), "{record}");
}

#[test]
fn a_deleted_starter_file_is_not_recreated_by_update() {
    let repo = copier_template();
    repo.file("template/managed.txt.jinja", "managed {{ name }} v2\n")
        .tag("v1.1.0");
    let project = copier_project(&repo, "v1.0.0");
    let dir = project.path();
    import(dir).code(0);
    commit_project(dir, "import");
    rootstock()
        .args(["update", "--defaults"])
        .current_dir(dir)
        .assert()
        .code(0);
    assert_eq!(
        fs::read_to_string(dir.join("managed.txt")).unwrap(),
        "managed shop v2\n"
    );
    assert!(!dir.join("start.txt").exists());
    assert!(!dir.join(".copier-answers.yml").exists());
}

#[test]
fn update_after_import_walks_rootstock_and_copier_tags() {
    let repo = copier_template();
    // v1.1.0: converted (rootstock.toml next to copier.yml).
    import(repo.path()).code(0);
    repo.file("template/managed.txt.jinja", "managed {{ name }} v2\n")
        .tag("v1.1.0");
    // v1.2.0: Copier format only.
    repo.remove("rootstock.toml")
        .file(ANSWERS_FILE, ANSWERS_BODY)
        .file("template/managed.txt.jinja", "managed {{ name }} v3\n")
        .tag("v1.2.0");
    let project = copier_project(&repo, "v1.0.0");
    let dir = project.path();
    import(dir).code(0);
    commit_project(dir, "import");

    rootstock()
        .args(["update", "--to", "v1.1.0", "--defaults"])
        .current_dir(dir)
        .assert()
        .code(0);
    assert_eq!(
        fs::read_to_string(dir.join("managed.txt")).unwrap(),
        "managed shop v2\n"
    );
    commit_project(dir, "update 1.1");
    rootstock()
        .args(["update", "--to", "v1.2.0", "--defaults"])
        .current_dir(dir)
        .assert()
        .code(0);
    assert_eq!(
        fs::read_to_string(dir.join("managed.txt")).unwrap(),
        "managed shop v3\n"
    );
    assert!(!dir.join(".copier-answers.yml").exists());
    let record = fs::read_to_string(dir.join(".rootstock.toml")).unwrap();
    assert!(!record.contains("copier-answers"), "{record}");
}

#[test]
fn a_commit_that_is_not_a_tag_is_refused() {
    let repo = copier_template();
    let project = copier_project(&repo, "v1.0.0-3-gabc1234");
    // `v1.0.0-3-gabc1234` is a valid semver pre-release, so it is the checkout that refuses it.
    imports_nothing(project.path(), "v1.0.0-3-gabc1234");
}

#[test]
fn an_unknown_answer_key_is_refused() {
    let repo = copier_template();
    let project = copier_project(&repo, "v1.0.0");
    let path = project.path().join(".copier-answers.yml");
    let mut text = fs::read_to_string(&path).unwrap();
    text.push_str("bogus: 1\n");
    fs::write(&path, text).unwrap();
    commit_project(project.path(), "extra");
    imports_nothing(
        project.path(),
        ".copier-answers.yml: `bogus` is not a question of the template at v1.0.0",
    );
}

#[test]
fn a_dirty_tree_is_refused() {
    let repo = copier_template();
    let project = copier_project(&repo, "v1.0.0");
    fs::write(project.path().join("managed.txt"), "edited\n").unwrap();
    imports_nothing(project.path(), "uncommitted changes");
}

#[test]
fn a_directory_that_is_not_a_git_tree_is_refused() {
    let repo = copier_template();
    let tmp = TempDir::new().unwrap();
    fs::write(
        tmp.path().join(".copier-answers.yml"),
        format!("_commit: v1.0.0\n_src_path: {}\nname: shop\n", repo.url()),
    )
    .unwrap();
    imports_nothing(tmp.path(), "not a git work tree");
}

#[test]
fn an_existing_record_is_refused() {
    let repo = copier_template();
    let project = copier_project(&repo, "v1.0.0");
    fs::write(project.path().join(".rootstock.toml"), "schema = 1\n").unwrap();
    commit_project(project.path(), "record");
    imports_nothing(project.path(), ".rootstock.toml already exists");
}

/// A committed Copier project that is also a Copier template (`copier.yml` next to the answers).
fn template_and_project(copier_yml: &str, commit_line: &str) -> (TemplateRepo, TempDir) {
    let repo = copier_template();
    let project = copier_project(&repo, commit_line);
    fs::write(project.path().join("copier.yml"), copier_yml).unwrap();
    fs::create_dir(project.path().join("template")).unwrap();
    fs::write(project.path().join("template/hello.txt.jinja"), "hello\n").unwrap();
    commit_project(project.path(), "both");
    (repo, project)
}

#[test]
fn a_template_and_a_project_at_once_are_both_imported() {
    let (_repo, project) = template_and_project(SMALL, "v1.0.0");
    let dir = project.path();
    let managed = fs::read(dir.join("managed.txt")).unwrap();
    import(dir)
        .code(0)
        .stdout(predicate::str::is_empty())
        .stderr(predicate::str::contains("wrote rootstock.toml"))
        .stderr(predicate::str::contains("wrote .rootstock.toml"))
        .stderr(predicate::str::contains("removed .copier-answers.yml"));
    assert!(dir.join("rootstock.toml").exists());
    assert!(dir.join(".rootstock.toml").exists());
    assert!(dir.join("copier.yml").exists());
    assert!(!dir.join(".copier-answers.yml").exists());
    assert_eq!(fs::read(dir.join("managed.txt")).unwrap(), managed);
    let record = fs::read_to_string(dir.join(".rootstock.toml")).unwrap();
    assert!(record.contains("version = \"v1.0.0\""), "{record}");
    assert!(record.contains("name = \"shop\""), "{record}");
}

#[test]
fn an_unsupported_template_side_writes_neither_side() {
    let (_repo, project) = template_and_project(
        "_subdirectory: template\n_jinja_extensions: [x]\nq:\n  type: int\n",
        "v1.0.0",
    );
    let dir = project.path();
    let before = snapshot(dir);
    import(dir)
        .code(1)
        .stdout(predicate::str::is_empty())
        .stderr(predicate::str::contains("_jinja_extensions"));
    assert_eq!(snapshot(dir), before);
    assert!(!dir.join("rootstock.toml").exists());
    assert!(!dir.join(".rootstock.toml").exists());
    assert!(dir.join(".copier-answers.yml").exists());
}

#[test]
fn a_commit_that_is_not_a_tag_writes_no_template_side() {
    let (_repo, project) = template_and_project(SMALL, "HEAD");
    let dir = project.path();
    let before = snapshot(dir);
    import(dir)
        .code(1)
        .stdout(predicate::str::is_empty())
        .stderr(predicate::str::contains("must be a tag vX.Y.Z"));
    assert_eq!(snapshot(dir), before);
    assert!(!dir.join("rootstock.toml").exists());
}

#[test]
fn a_parent_that_does_not_render_writes_no_template_side() {
    // The tag is valid, so the project side fails late: at the render of the parent.
    let repo = TemplateRepo::new();
    repo.file("copier.yml", PROJECT_COPIER)
        .file("template/bad.txt.jinja", "{% if %}\n")
        .tag("v1.0.0");
    let project = copier_project(&repo, "v1.0.0");
    let dir = project.path();
    fs::write(dir.join("copier.yml"), SMALL).unwrap();
    fs::create_dir(dir.join("template")).unwrap();
    fs::write(dir.join("template/hello.txt.jinja"), "hello\n").unwrap();
    commit_project(dir, "both");
    let before = snapshot(dir);
    import(dir)
        .code(1)
        .stdout(predicate::str::is_empty())
        .stderr(predicate::str::contains("cannot render template version"));
    assert_eq!(snapshot(dir), before);
    assert!(!dir.join("rootstock.toml").exists());
    assert!(!dir.join(".rootstock.toml").exists());
    assert!(dir.join(".copier-answers.yml").exists());
}

#[test]
fn two_copier_files_and_the_answers_file_are_still_an_error() {
    let (_repo, project) = template_and_project(SMALL, "v1.0.0");
    fs::write(project.path().join("copier.yaml"), SMALL).unwrap();
    commit_project(project.path(), "yaml");
    imports_nothing(project.path(), "both copier.yml and copier.yaml");
}

#[test]
fn nothing_to_import_names_all_three_files() {
    let tmp = TempDir::new().unwrap();
    import(tmp.path())
        .code(1)
        .stderr(predicate::str::contains("copier.yml"))
        .stderr(predicate::str::contains("copier.yaml"))
        .stderr(predicate::str::contains(".copier-answers.yml"));
    assert_eq!(fs::read_dir(tmp.path()).unwrap().count(), 0);
}

#[test]
fn unsupported_features_in_the_tag_are_listed() {
    let repo = TemplateRepo::new();
    repo.file(
        "copier.yml",
        "_subdirectory: template\n_jinja_extensions: [x]\nname:\n  type: str\n  default: demo\n",
    )
    .file("template/a.txt", "x")
    .tag("v1.0.0");
    let project = copier_project(&repo, "v1.0.0");
    imports_nothing(
        project.path(),
        "copier.yml uses features rootstock cannot convert:",
    );
    import(project.path()).stderr(predicate::str::contains("_jinja_extensions"));
}

#[test]
fn a_bad_answers_file_is_refused() {
    let repo = copier_template();
    let project = copier_project(&repo, "v1.0.0");
    fs::write(project.path().join(".copier-answers.yml"), "name: shop\n").unwrap();
    commit_project(project.path(), "broken");
    imports_nothing(project.path(), "_src_path");
}

#[test]
fn new_refuses_a_copier_only_tag() {
    let repo = copier_template();
    let tmp = TempDir::new().unwrap();
    let project = tmp.path().join("proj");
    rootstock()
        .args(["new", &repo.url()])
        .arg(&project)
        .arg("--defaults")
        .assert()
        .code(1)
        .stderr(predicate::str::contains("rootstock import-copier"));
    assert!(!project.exists());
}

#[test]
fn an_unsupported_target_tag_stops_update_before_any_write() {
    let repo = copier_template();
    repo.file("copier.yml", &format!("{PROJECT_COPIER}_migrations: []\n"))
        .tag("v1.1.0");
    let project = copier_project(&repo, "v1.0.0");
    let dir = project.path();
    import(dir).code(0);
    commit_project(dir, "import");
    let before = snapshot(dir);
    rootstock()
        .args(["update", "--defaults"])
        .current_dir(dir)
        .assert()
        .code(1)
        .stderr(predicate::str::contains("_migrations"));
    assert_eq!(snapshot(dir), before);
    assert_eq!(git_in(dir, &["status", "--porcelain"]), "");
}

#[test]
fn only_the_two_files_differ_after_an_import() {
    let repo = copier_template();
    let project = copier_project(&repo, "v1.0.0");
    let dir = project.path();
    let before = snapshot(dir);
    import(dir).code(0);
    let after = snapshot(dir);
    let unchanged: Vec<_> = before
        .iter()
        .filter(|(name, _)| name != ".copier-answers.yml")
        .cloned()
        .collect();
    let rest: Vec<_> = after
        .iter()
        .filter(|(name, _)| name != ".rootstock.toml")
        .cloned()
        .collect();
    assert_eq!(rest, unchanged);
}

#[test]
fn a_ready_answers_file_in_the_template_root_is_not_inventoried() {
    let repo = TemplateRepo::new();
    repo.file(
        "copier.yml",
        "_templates_suffix: .jinja\nname:\n  type: str\n  default: demo\n",
    )
    .file("{{ _copier_conf.answers_file }}.jinja", ANSWERS_BODY)
    .file(
        ".copier-answers.yml",
        "_commit: v0.1.0\n_src_path: elsewhere\n",
    )
    .file("managed.txt.jinja", "managed {{ name }} v1\n")
    .tag("v1.0.0");
    repo.file("managed.txt.jinja", "managed {{ name }} v2\n")
        .tag("v1.1.0");
    let project = copier_project(&repo, "v1.0.0");
    let dir = project.path();
    import(dir).code(0);
    let record = fs::read_to_string(dir.join(".rootstock.toml")).unwrap();
    assert!(!record.contains("copier-answers"), "{record}");
    commit_project(dir, "import");
    rootstock()
        .args(["update", "--defaults"])
        .current_dir(dir)
        .assert()
        .code(0)
        .stdout(predicate::str::is_empty());
    assert!(!dir.join(".copier-answers.yml").exists());
}

#[test]
fn a_path_the_project_holds_is_blocked() {
    let repo = TemplateRepo::new();
    repo.file("copier.yml", PROJECT_COPIER)
        .file("template/a/b.jinja", "b\n")
        .file("template/managed.txt.jinja", "managed\n")
        .tag("v1.0.0");
    let project = copier_project(&repo, "v1.0.0");
    let dir = project.path();
    fs::write(dir.join("a"), "a plain file\n").unwrap();
    commit_project(dir, "a is a file");
    rootstock()
        .arg("import-copier")
        .current_dir(dir)
        .assert()
        .code(2)
        .stdout("blocked a/b\n")
        .stderr(predicate::str::contains("1 conflict"));
    let record = fs::read_to_string(dir.join(".rootstock.toml")).unwrap();
    assert!(!record.contains("a/b"), "{record}");
    assert!(record.contains("\"managed.txt\""), "{record}");
    assert!(!dir.join(".copier-answers.yml").exists());
    assert_eq!(fs::read_to_string(dir.join("a")).unwrap(), "a plain file\n");
}

#[test]
fn project_and_template_edits_merge_after_an_import() {
    let repo = TemplateRepo::new();
    repo.file("copier.yml", PROJECT_COPIER)
        .file(ANSWERS_FILE, ANSWERS_BODY)
        .file(
            "template/managed.txt.jinja",
            "{{ name }}\nb\nc\nd\ne\nf\ng\n",
        )
        .tag("v1.0.0");
    repo.file(
        "template/managed.txt.jinja",
        "{{ name }}\nb\nc\nd\ne\nf\nG-template\n",
    )
    .tag("v1.1.0");
    let project = copier_project(&repo, "v1.0.0");
    let dir = project.path();
    fs::write(dir.join("managed.txt"), "shop\nb\nc\nd\ne\nf\ng\n").unwrap();
    commit_project(dir, "render");
    import(dir).code(0);
    commit_project(dir, "import");
    fs::write(dir.join("managed.txt"), "shop\nB-project\nc\nd\ne\nf\ng\n").unwrap();
    commit_project(dir, "project edit");
    rootstock()
        .args(["update", "--defaults"])
        .current_dir(dir)
        .assert()
        .code(0);
    assert_eq!(
        fs::read_to_string(dir.join("managed.txt")).unwrap(),
        "shop\nB-project\nc\nd\ne\nf\nG-template\n"
    );
}

#[test]
fn rootstock_toml_on_the_tag_wins_over_the_conversion() {
    let repo = TemplateRepo::new();
    repo.file("copier.yml", PROJECT_COPIER)
        .file(
            "rootstock.toml",
            "schema = 1\nroot = \"template\"\nsuffix = \".jinja\"\n[files]\nstarter = [\"/managed.txt\"]\n[[questions]]\nname = \"name\"\ntype = \"str\"\ndefault = \"demo\"\n",
        )
        .file("template/managed.txt.jinja", "managed {{ name }}\n")
        .tag("v1.0.0");
    let project = copier_project(&repo, "v1.0.0");
    let dir = project.path();
    import(dir).code(0);
    let record = fs::read_to_string(dir.join(".rootstock.toml")).unwrap();
    assert!(record.contains("\"managed.txt\" = \"starter\""), "{record}");
}

#[test]
fn answers_failing_the_type_or_the_validator_are_refused() {
    let repo = TemplateRepo::new();
    repo.file(
        "copier.yml",
        "_subdirectory: template\nflag:\n  type: bool\n  default: true\nname:\n  type: str\n  default: demo\n  validator: \"{% if name == 'bad' %}no bad names{% endif %}\"\n",
    )
    .file("template/a.txt", "x")
    .tag("v1.0.0");
    let with_answers = |extra: &str| {
        let project = copier_project(&repo, "v1.0.0");
        let path = project.path().join(".copier-answers.yml");
        let mut text = fs::read_to_string(&path).unwrap();
        text.push_str(extra);
        fs::write(&path, text).unwrap();
        commit_project(project.path(), "answers");
        project
    };
    // A wrong type.
    let project = with_answers("flag: maybe\n");
    imports_nothing(project.path(), ".copier-answers.yml answers at v1.0.0");
    import(project.path()).stderr(predicate::str::contains("flag"));
    // A validator that rejects the answer (`name: shop` is already in the file).
    let project = with_answers("");
    let path = project.path().join(".copier-answers.yml");
    let text = fs::read_to_string(&path)
        .unwrap()
        .replace("name: shop", "name: bad");
    fs::write(&path, text).unwrap();
    commit_project(project.path(), "bad name");
    imports_nothing(project.path(), ".copier-answers.yml answers at v1.0.0");
    import(project.path()).stderr(predicate::str::contains("no bad names"));
    // The control: right answers on the same template import.
    let project = with_answers("flag: false\n");
    import(project.path()).code(0);
    assert!(project.path().join(".rootstock.toml").exists());
}

#[cfg(unix)]
#[test]
fn an_answers_file_that_is_a_link_is_refused() {
    let repo = copier_template();
    let project = copier_project(&repo, "v1.0.0");
    let dir = project.path();
    fs::rename(dir.join(".copier-answers.yml"), dir.join("real.yml")).unwrap();
    std::os::unix::fs::symlink("real.yml", dir.join(".copier-answers.yml")).unwrap();
    commit_project(dir, "link");
    imports_nothing(dir, "regular file");
}

#[test]
fn hooks_do_not_run_on_import() {
    let repo = TemplateRepo::new();
    repo.file(
        "copier.yml",
        &format!("{PROJECT_COPIER}_tasks:\n  - \": > marker.txt\"\n"),
    )
    .file("template/managed.txt.jinja", "managed\n")
    .tag("v1.0.0");
    let project = copier_project(&repo, "v1.0.0");
    import(project.path()).code(0);
    assert!(!project.path().join("marker.txt").exists());
}

#[test]
fn a_bad_commit_names_the_key_and_the_file() {
    let repo = copier_template();
    let project = copier_project(&repo, "HEAD");
    imports_nothing(
        project.path(),
        "`_commit` in .copier-answers.yml is `HEAD`: it must be a tag vX.Y.Z",
    );
}

#[test]
fn help_describes_both_sides() {
    rootstock()
        .args(["import-copier", "--help"])
        .assert()
        .code(0)
        .stdout(predicate::str::contains(".copier-answers.yml"))
        .stdout(predicate::str::contains("copier.yml"))
        .stdout(predicate::str::contains("clean git"));
}
