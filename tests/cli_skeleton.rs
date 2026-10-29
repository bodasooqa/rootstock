use assert_cmd::Command;
use predicates::prelude::*;

#[test]
fn bogus_flag_exits_1_with_message_on_stderr() {
    Command::cargo_bin("rootstock")
        .unwrap()
        .arg("--bogus")
        .assert()
        .code(1)
        .stdout(predicate::str::is_empty())
        .stderr(predicate::str::contains("--bogus"));
}

#[test]
fn version_exits_0_with_version_on_stdout() {
    Command::cargo_bin("rootstock")
        .unwrap()
        .arg("--version")
        .assert()
        .code(0)
        .stdout(predicate::str::contains(env!("CARGO_PKG_VERSION")));
}
