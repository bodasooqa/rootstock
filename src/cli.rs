//! Command-line interface: argument parsing and command output.

use std::path::PathBuf;

use anyhow::Result;
use clap::error::ErrorKind;
use clap::{Args, Parser, Subcommand, ValueEnum};

use crate::commands::new::{self, NewOptions};
use crate::commands::update::{self, UpdateOptions};
use crate::commands::{CommonOptions, Report};
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
        Command::New(args) => {
            let report = new::run(&NewOptions {
                template: args.template,
                dir: args.dir,
                version: args.version,
                git: args.git,
                common: args.common.options(),
            })?;
            (report, args.common.format)
        }
        Command::Update(args) => {
            let report = update::run(
                &std::env::current_dir()?,
                &UpdateOptions {
                    to: args.to,
                    common: args.common.options(),
                },
            )?;
            (report, args.common.format)
        }
    };
    print_report(&report, format)?;
    if report.conflicts.is_empty() {
        Ok(Outcome::Clean)
    } else {
        eprintln!("{} conflicts", report.conflicts.len());
        Ok(Outcome::Conflicts(report.conflicts))
    }
}

fn print_report(report: &Report, format: Format) -> Result<()> {
    match format {
        Format::Text => {
            for c in &report.conflicts {
                println!("{} {}", c.kind.as_str(), c.path);
            }
        }
        Format::Json => {
            let conflicts: Vec<_> = report
                .conflicts
                .iter()
                .map(|c| serde_json::json!({"path": c.path, "kind": c.kind.as_str()}))
                .collect();
            let value = serde_json::json!({"version": report.version, "conflicts": conflicts});
            println!("{}", serde_json::to_string(&value)?);
        }
    }
    Ok(())
}
