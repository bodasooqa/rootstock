mod support;

use rootstock::source::{Source, checkout, latest, parse_source, tags};
use support::TemplateRepo;

#[test]
fn parses_gh_shorthand() {
    assert_eq!(
        parse_source("gh:o/r"),
        Source::Git("https://github.com/o/r.git".to_string())
    );
    assert_eq!(
        parse_source("https://example.com/x.git"),
        Source::Git("https://example.com/x.git".to_string())
    );
}

#[test]
fn existing_dir_is_local() {
    let repo = TemplateRepo::new();
    assert_eq!(
        parse_source(&repo.url()),
        Source::Local(repo.path().to_path_buf())
    );
}

#[test]
fn latest_skips_prereleases() {
    let repo = TemplateRepo::new();
    repo.file("a", "1")
        .tag("v1.0.0")
        .file("a", "2")
        .tag("v1.2.0")
        .file("a", "3")
        .tag("v2.0.0-rc.1")
        .file("a", "4")
        .tag("foo");
    let src = parse_source(&repo.url());
    let listed: Vec<String> = tags(&src).unwrap().into_iter().map(|(_, t)| t).collect();
    assert_eq!(listed, vec!["v1.0.0", "v1.2.0", "v2.0.0-rc.1"]);
    assert_eq!(latest(&src).unwrap(), "v1.2.0");
}

#[test]
fn latest_without_release_errors() {
    let repo = TemplateRepo::new();
    repo.file("a", "1").tag("v1.0.0-rc.1");
    assert!(latest(&parse_source(&repo.url())).is_err());
}

#[test]
fn checkout_by_tag_and_by_sha() {
    let repo = TemplateRepo::new();
    repo.file("a.txt", "one").tag("v1.0.0");
    repo.file("a.txt", "two").tag("v2.0.0");
    let src = parse_source(&repo.url());

    let by_tag = checkout(&src, "v1.0.0").unwrap();
    assert_eq!(
        std::fs::read_to_string(by_tag.path().join("a.txt")).unwrap(),
        "one"
    );
    assert_eq!(by_tag.commit, repo.commit("v1.0.0"));

    let sha = repo.commit("v2.0.0");
    let by_sha = checkout(&src, &sha).unwrap();
    assert_eq!(
        std::fs::read_to_string(by_sha.path().join("a.txt")).unwrap(),
        "two"
    );
    assert_eq!(by_sha.commit, sha);
}

#[test]
fn local_head_is_working_tree() {
    let repo = TemplateRepo::new();
    repo.file("a.txt", "one").tag("v1.0.0");
    repo.file("new.txt", "uncommitted");
    let src = parse_source(&repo.url());
    let co = checkout(&src, "HEAD").unwrap();
    assert_eq!(co.path(), repo.path());
    assert!(co.path().join("new.txt").exists());
    assert_eq!(co.commit, repo.commit("HEAD"));
}

#[test]
fn missing_commit_errors() {
    let repo = TemplateRepo::new();
    repo.file("a.txt", "one").tag("v1.0.0");
    let sha = "0123456789abcdef0123456789abcdef01234567";
    let err = checkout(&parse_source(&repo.url()), sha).unwrap_err();
    assert!(err.to_string().contains(sha), "{err}");
}
