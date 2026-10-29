//! Command-line interface. Subcommands arrive in later tasks.

use clap::Parser;

/// Result of a command that ran to completion.
#[derive(Debug)]
pub enum Outcome {
    /// Everything applied without conflicts (exit code 0).
    Clean,
    /// Done, but some paths hold conflicts (exit code 2).
    Conflicts(Vec<String>),
}

/// Create a project from a versioned git template and update it later.
#[derive(Debug, Parser)]
#[command(name = "rootstock", version, about)]
struct Cli {}

/// Parses the process arguments and runs the requested command.
pub fn run() -> anyhow::Result<Outcome> {
    let _cli = Cli::parse();
    Ok(Outcome::Clean)
}
