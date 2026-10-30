//! Command-line interface: argument parsing and command output.

use std::path::PathBuf;

use std::io::Write;

use anyhow::{Context, Result, bail};
use clap::error::ErrorKind;
use clap::{Args, Parser, Subcommand, ValueEnum};

use crate::commands::check;
use crate::commands::import_copier;
use crate::commands::new::{self, NewOptions};
use crate::commands::status::{self, Status};
use crate::commands::update::{self, UpdateOptions};
use crate::commands::{CommonOptions, HookFailed, Report};
use crate::plan::Conflict;

/// Result of a command that ran to completion.
#[derive(Debug)]
pub enum Outcome {
    /// Everything applied without conflicts (exit code 0).
    Clean,
    /// Done, but some paths hold conflicts (exit code 2).
    Conflicts(Vec<Conflict>),
}

/// Create a project from a versioned git template and update it later.
#[derive(Debug, Parser)]
#[command(name = "rootstock", version, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Create a project from a template.
    New(NewArgs),
    /// Update the project in the current directory to a newer template version.
    Update(UpdateArgs),
    /// Show the current and the latest template version.
    Status(StatusArgs),
    /// Fail if conflict markers or side files are left in the project.
    Check,
    /// Convert the Copier template in the current directory: write rootstock.toml from copier.yml.
    ///
    /// Stops with a list and writes nothing if the template uses anything rootstock
    /// cannot convert. The Copier answers-file template is removed after the conversion.
    ImportCopier,
}

#[derive(Debug, Args)]
struct StatusArgs {
    /// Output format.
    #[arg(long, value_enum, default_value = "text")]
    format: Format,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum Format {
    Text,
    Json,
}

/// Flags shared by `new` and `update`.
#[derive(Debug, Args)]
struct CommonArgs {
    /// Answer a question: name=value (repeatable).
    #[arg(long = "data", value_name = "NAME=VALUE")]
    data: Vec<String>,
    /// TOML file with answers; --data wins over it.
    #[arg(long, value_name = "FILE")]
    data_file: Option<PathBuf>,
    /// Use defaults instead of asking.
    #[arg(long)]
    defaults: bool,
    /// Run template hooks without asking.
    #[arg(long)]
    trust: bool,
    /// Output format for conflicts.
    #[arg(long, value_enum, default_value = "text")]
    format: Format,
}

impl CommonArgs {
    fn options(&self) -> CommonOptions {
        CommonOptions {
            data: self.data.clone(),
            data_file: self.data_file.clone(),
            defaults: self.defaults,
            trust: self.trust,
        }
    }
}

#[derive(Debug, Args)]
struct NewArgs {
    /// Template source: URL, gh:owner/repo or a local path.
    template: String,
    /// Directory to create the project in.
    dir: PathBuf,
    /// Template version (tag); the latest release by default.
    #[arg(long, value_name = "VERSION")]
    version: Option<String>,
    /// Directory of the template inside the source (holds `rootstock.toml`).
    #[arg(long, value_name = "PATH")]
    subdir: Option<String>,
    /// Run `git init` in an empty directory.
    #[arg(long)]
    git: bool,
    #[command(flatten)]
    common: CommonArgs,
}

#[derive(Debug, Args)]
struct UpdateArgs {
    /// Target template version (tag); the latest release by default.
    #[arg(long, value_name = "VERSION")]
    to: Option<String>,
    #[command(flatten)]
    common: CommonArgs,
}

/// Command-line arguments were rejected; clap has already printed the message,
/// so the caller must not print it again. Maps to exit code 1.
#[derive(Debug, thiserror::Error)]
#[error("invalid command-line arguments")]
pub struct UsageError;

/// Parses the process arguments and runs the requested command.
///
/// clap's own `parse()` exits with code 2 on bad arguments, which would
/// collide with "done with conflicts", so errors are reported as `UsageError`.
pub fn run() -> Result<Outcome> {
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(e) => {
            let display_only =
                matches!(e.kind(), ErrorKind::DisplayHelp | ErrorKind::DisplayVersion);
            // Best effort: nothing useful can be done if the stream is closed.
            let _ = e.print();
            return if display_only {
                Ok(Outcome::Clean)
            } else {
                Err(UsageError.into())
            };
        }
    };
    let (report, format) = match cli.command {
        Command::Status(args) => {
            let status = status::run(&current_dir()?)?;
            return print_status(&status, args.format).map(|()| Outcome::Clean);
        }
        Command::Check => {
            let lines = check::run(&current_dir()?)?;
            if lines.is_empty() {
                return Ok(Outcome::Clean);
            }
            let mut out = lines.join("\n");
            out.push('\n');
            write_stdout(&out)?;
            // Reported as an error so that main maps it to exit code 1.
            bail!("check failed: {} problem(s)", lines.len());
        }
        Command::ImportCopier => {
            import_copier::run(&current_dir()?, &mut std::io::stderr().lock())?;
            return Ok(Outcome::Clean);
        }
        Command::New(args) => {
            let report = new::run(&NewOptions {
                template: args.template,
                dir: args.dir,
                version: args.version,
                subdir: args.subdir,
                git: args.git,
                common: args.common.options(),
            });
            (listed(report, args.common.format)?, args.common.format)
        }
        Command::Update(args) => {
            let report = update::run(
                &current_dir()?,
                &UpdateOptions {
                    to: args.to,
                    common: args.common.options(),
                },
            );
            (listed(report, args.common.format)?, args.common.format)
        }
    };
    print_report(&report, format)?;
    if report.conflicts.is_empty() {
        Ok(Outcome::Clean)
    } else {
        let count = report.conflicts.len();
        eprintln!("{count} conflict{}", if count == 1 { "" } else { "s" });
        Ok(Outcome::Conflicts(report.conflicts))
    }
}

/// Passes a report through; when a hook failed after the files were written,
/// lists the conflicts on disk before returning the error.
fn listed(result: Result<Report>, format: Format) -> Result<Report> {
    let err = match result {
        Ok(report) => return Ok(report),
        Err(err) => err,
    };
    if let Some(failed) = err.downcast_ref::<HookFailed>() {
        print_report(&failed.report, format)?;
    }
    Err(err)
}

fn print_report(report: &Report, format: Format) -> Result<()> {
    let mut out = String::new();
    match format {
        Format::Text => {
            for c in &report.conflicts {
                out.push_str(&format!("{} {}\n", c.kind.as_str(), c.path));
            }
        }
        Format::Json => {
            let conflicts: Vec<_> = report
                .conflicts
                .iter()
                .map(|c| serde_json::json!({"path": c.path, "kind": c.kind.as_str()}))
                .collect();
            let value = serde_json::json!({"version": report.version, "conflicts": conflicts});
            out.push_str(&serde_json::to_string(&value)?);
            out.push('\n');
        }
    }
    write_stdout(&out)
}

fn current_dir() -> Result<PathBuf> {
    std::env::current_dir().context("cannot determine the current directory")
}

fn print_status(status: &Status, format: Format) -> Result<()> {
    let out = match format {
        Format::Text => format!(
            "current {}\nlatest {}\nmajor {}\n",
            status.current, status.latest, status.major
        ),
        Format::Json => {
            let value = serde_json::json!({
                "current": status.current,
                "latest": status.latest,
                "major": status.major,
            });
            format!("{}\n", serde_json::to_string(&value)?)
        }
    };
    write_stdout(&out)
}

fn write_stdout(out: &str) -> Result<()> {
    // The work is already done: a closed pipe must not change the exit code.
    match std::io::stdout().lock().write_all(out.as_bytes()) {
        Err(e) if e.kind() != std::io::ErrorKind::BrokenPipe => Err(e.into()),
        _ => Ok(()),
    }
}
