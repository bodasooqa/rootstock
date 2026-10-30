//! Runs template hooks in the project directory, gated by trust.

use std::path::Path;
use std::process::Stdio;

use crate::config::{Hook, Phase};

fn select(hooks: &[Hook], phase: Phase) -> Vec<&Hook> {
    hooks.iter().filter(|h| h.when.contains(&phase)).collect()
}

/// Checks that the hooks for `phase` may run: nothing selected, `trust`, or an
/// interactive confirmation. Runs nothing; call it before any file is written.
pub fn check_trust(
    hooks: &[Hook],
    phase: Phase,
    trust: bool,
    interactive: bool,
) -> anyhow::Result<()> {
    let selected = select(hooks, phase);
    if selected.is_empty() || trust {
        return Ok(());
    }
    let listing: Vec<String> = selected.iter().map(|h| format!("  {}", h.run)).collect();
    let listing = listing.join("\n");
    if !interactive {
        anyhow::bail!("template has hooks; rerun with --trust\n{listing}");
    }
    let prompt = format!("The template wants to run these commands:\n{listing}\nRun them?");
    match inquire::Confirm::new(&prompt).with_default(false).prompt() {
        Ok(true) => Ok(()),
        Ok(false)
        | Err(
            inquire::InquireError::OperationCanceled | inquire::InquireError::OperationInterrupted,
        ) => anyhow::bail!("hooks declined; nothing was run"),
        Err(e) => Err(anyhow::Error::new(e).context("could not ask for hook confirmation")),
    }
}

/// Runs the hooks selected for `phase` without asking, in order, stopping at the
/// first failure. The caller must have passed `check_trust`.
pub fn run_trusted(hooks: &[Hook], phase: Phase, project: &Path) -> anyhow::Result<()> {
    for hook in select(hooks, phase) {
        run_one(hook, project)?;
    }
    Ok(())
}

/// `check_trust` followed by `run_trusted`.
pub fn run_hooks(
    hooks: &[Hook],
    phase: Phase,
    project: &Path,
    trust: bool,
    interactive: bool,
) -> anyhow::Result<()> {
    check_trust(hooks, phase, trust, interactive)?;
    run_trusted(hooks, phase, project)
}

fn run_one(hook: &Hook, project: &Path) -> anyhow::Result<()> {
    // Hook output goes to stderr: stdout carries only command output.
    let status = std::process::Command::new("sh")
        .arg("-c")
        .arg(&hook.run)
        .current_dir(project)
        .stdin(Stdio::null())
        .stdout(Stdio::from(std::io::stderr()))
        .stderr(Stdio::from(std::io::stderr()))
        .status()
        .map_err(|e| anyhow::anyhow!("hook `{}` failed: {e}", hook.run))?;
    if status.success() {
        return Ok(());
    }
    match status.code() {
        Some(code) => anyhow::bail!("hook `{}` failed: exit status {code}", hook.run),
        None => anyhow::bail!("hook `{}` failed: killed by signal", hook.run),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hook(run: &str, when: &[Phase]) -> Hook {
        Hook {
            run: run.to_string(),
            when: when.to_vec(),
        }
    }

    #[test]
    fn hooks_need_trust_noninteractive() {
        let dir = tempfile::tempdir().unwrap();
        let hooks = [hook("touch marker", &[Phase::New])];
        let err = run_hooks(&hooks, Phase::New, dir.path(), false, false)
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("template has hooks; rerun with --trust"),
            "{err}"
        );
        assert!(err.contains("touch marker"), "{err}");
        assert!(!dir.path().join("marker").exists());
    }

    #[test]
    fn hooks_run_in_order_in_project_dir() {
        let dir = tempfile::tempdir().unwrap();
        let hooks = [
            hook("pwd > where; echo one > order", &[Phase::New]),
            hook("echo two >> order", &[Phase::New, Phase::Update]),
        ];
        run_hooks(&hooks, Phase::New, dir.path(), true, false).unwrap();
        let order = std::fs::read_to_string(dir.path().join("order")).unwrap();
        assert_eq!(order, "one\ntwo\n");
        let where_ = std::fs::read_to_string(dir.path().join("where")).unwrap();
        let expected = std::fs::canonicalize(dir.path()).unwrap();
        assert_eq!(std::fs::canonicalize(where_.trim()).unwrap(), expected);
    }

    #[test]
    fn failing_hook_reports_command() {
        let dir = tempfile::tempdir().unwrap();
        let hooks = [
            hook("exit 3", &[Phase::New]),
            hook("touch after", &[Phase::New]),
        ];
        let err = run_hooks(&hooks, Phase::New, dir.path(), true, false)
            .unwrap_err()
            .to_string();
        assert!(err.contains("hook `exit 3` failed: exit status 3"), "{err}");
        assert!(!dir.path().join("after").exists());
    }

    #[test]
    fn check_trust_noninteractive_errors_and_runs_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let hooks = [hook("touch marker", &[Phase::New])];
        let err = check_trust(&hooks, Phase::New, false, false)
            .unwrap_err()
            .to_string();
        assert!(err.contains("rerun with --trust"), "{err}");
        assert!(!dir.path().join("marker").exists());
        check_trust(&hooks, Phase::New, true, false).unwrap();
        check_trust(&hooks, Phase::Update, false, false).unwrap();
    }

    #[test]
    fn hooks_filtered_by_phase() {
        let dir = tempfile::tempdir().unwrap();
        let hooks = [
            hook("touch new_only", &[Phase::New]),
            hook("touch update_only", &[Phase::Update]),
        ];
        run_hooks(&hooks, Phase::Update, dir.path(), true, false).unwrap();
        assert!(!dir.path().join("new_only").exists());
        assert!(dir.path().join("update_only").exists());
    }

    #[test]
    fn no_hooks_no_trust_needed() {
        let dir = tempfile::tempdir().unwrap();
        run_hooks(&[], Phase::New, dir.path(), false, false).unwrap();
        let hooks = [hook("touch x", &[Phase::Update])];
        run_hooks(&hooks, Phase::New, dir.path(), false, false).unwrap();
        assert!(!dir.path().join("x").exists());
    }
}
