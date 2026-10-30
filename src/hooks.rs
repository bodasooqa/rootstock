//! Runs template hooks in the project directory, gated by trust.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
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

/// The POSIX shell that runs hook commands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shell(PathBuf);

/// File name of the shell: `sh` on Unix, `sh.exe` from Git for Windows.
const SHELL_NAME: &str = if cfg!(windows) { "sh.exe" } else { "sh" };

/// Finds the shell for the hooks selected for `phase`; `None` when there are
/// none, so a command without hooks never looks for `sh`. Runs nothing; call
/// it before any file is written.
pub fn find_shell(hooks: &[Hook], phase: Phase) -> anyhow::Result<Option<Shell>> {
    if select(hooks, phase).is_empty() {
        return Ok(None);
    }
    if cfg!(not(windows)) {
        // The system resolves `sh` from `PATH` when the hook starts.
        return Ok(Some(Shell(PathBuf::from(SHELL_NAME))));
    }
    // Without git there is no exec path to search; `PATH` alone still counts.
    let exec_path = crate::source::git(&["--exec-path"]).ok();
    let path_var = std::env::var_os("PATH").unwrap_or_default();
    locate_shell(
        &path_var,
        exec_path.as_deref().map(Path::new),
        SHELL_NAME,
        &|candidate| candidate.is_file(),
    )
    .map(Some)
    .ok_or_else(|| {
        anyhow::anyhow!("hooks need `sh` from Git for Windows; none was found next to git")
    })
}

/// Looks for the shell `name` in the `PATH` entries, then in the `bin` and
/// `usr/bin` directories of every ancestor of git's exec path (Git for Windows
/// keeps `sh.exe` there, outside `PATH`). `is_file` tells what exists.
fn locate_shell(
    path_var: &OsStr,
    exec_path: Option<&Path>,
    name: &str,
    is_file: &dyn Fn(&Path) -> bool,
) -> Option<Shell> {
    let in_path = std::env::split_paths(path_var).map(|dir| dir.join(name));
    let beside_git = exec_path.into_iter().flat_map(|exec| {
        exec.ancestors()
            .flat_map(|dir| [dir.join("bin").join(name), dir.join("usr/bin").join(name)])
    });
    in_path
        .chain(beside_git)
        .find(|candidate| is_file(candidate))
        .map(Shell)
}

/// Runs the hooks selected for `phase` without asking, in order, stopping at the
/// first failure. The caller must have passed `check_trust` and holds the
/// `shell` that `find_shell` returned for the same hooks and phase.
pub fn run_trusted(
    hooks: &[Hook],
    phase: Phase,
    project: &Path,
    shell: Option<&Shell>,
) -> anyhow::Result<()> {
    for hook in select(hooks, phase) {
        let shell = shell.ok_or_else(|| anyhow::anyhow!("no shell was found for the hooks"))?;
        run_one(hook, project, shell)?;
    }
    Ok(())
}

/// `find_shell` and `check_trust` followed by `run_trusted`.
pub fn run_hooks(
    hooks: &[Hook],
    phase: Phase,
    project: &Path,
    trust: bool,
    interactive: bool,
) -> anyhow::Result<()> {
    let shell = find_shell(hooks, phase)?;
    check_trust(hooks, phase, trust, interactive)?;
    run_trusted(hooks, phase, project, shell.as_ref())
}

fn run_one(hook: &Hook, project: &Path, shell: &Shell) -> anyhow::Result<()> {
    // Hook output goes to stderr: stdout carries only command output.
    let status = std::process::Command::new(&shell.0)
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

    fn existing(files: &[PathBuf]) -> impl Fn(&Path) -> bool + '_ {
        move |candidate| files.iter().any(|f| f == candidate)
    }

    #[test]
    fn shell_is_taken_from_path_first() {
        let in_path = Path::new("/tools").join("sh.exe");
        let beside_git = Path::new("/g/Git").join("bin").join("sh.exe");
        let files = [in_path.clone(), beside_git];
        let path_var = std::env::join_paths(["/other", "/tools"]).unwrap();
        let exec = Path::new("/g/Git/mingw64/libexec/git-core");
        let found = locate_shell(&path_var, Some(exec), "sh.exe", &existing(&files));
        assert_eq!(found, Some(Shell(in_path)));
    }

    #[test]
    fn shell_is_searched_beside_git_up_the_exec_path() {
        let exec = Path::new("/g/Git/mingw64/libexec/git-core");
        let path_var = std::env::join_paths(["/other"]).unwrap();
        // `bin` wins over `usr/bin` in the same ancestor; a nearer ancestor wins.
        let bin = Path::new("/g/Git").join("bin").join("sh.exe");
        let usr = Path::new("/g/Git").join("usr/bin").join("sh.exe");
        let files = [usr.clone(), bin.clone()];
        let found = locate_shell(&path_var, Some(exec), "sh.exe", &existing(&files));
        assert_eq!(found, Some(Shell(bin)));
        let files = [usr.clone()];
        let found = locate_shell(&path_var, Some(exec), "sh.exe", &existing(&files));
        assert_eq!(found, Some(Shell(usr)));
        let near = Path::new("/g/Git/mingw64").join("bin").join("sh.exe");
        let files = [near.clone(), Path::new("/g/Git").join("bin").join("sh.exe")];
        let found = locate_shell(&path_var, Some(exec), "sh.exe", &existing(&files));
        assert_eq!(found, Some(Shell(near)));
    }

    #[test]
    fn missing_shell_is_none() {
        let path_var = std::env::join_paths(["/other"]).unwrap();
        let exec = Path::new("/g/Git/mingw64/libexec/git-core");
        assert_eq!(
            locate_shell(&path_var, Some(exec), "sh.exe", &|_| false),
            None
        );
        assert_eq!(locate_shell(&path_var, None, "sh.exe", &|_| false), None);
    }

    #[test]
    fn no_hooks_look_for_no_shell() {
        assert_eq!(find_shell(&[], Phase::New).unwrap(), None);
        let hooks = [hook("true", &[Phase::Update])];
        assert_eq!(find_shell(&hooks, Phase::New).unwrap(), None);
        assert!(find_shell(&hooks, Phase::Update).unwrap().is_some());
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
