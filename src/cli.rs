//! Command-line interface. Subcommands arrive in later tasks.

use clap::Parser;
use clap::error::ErrorKind;

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

/// Command-line arguments were rejected; clap has already printed the message,
/// so the caller must not print it again. Maps to exit code 1.
#[derive(Debug, thiserror::Error)]
#[error("invalid command-line arguments")]
pub struct UsageError;

/// Parses the process arguments and runs the requested command.
///
/// clap's own `parse()` exits with code 2 on bad arguments, which would
/// collide with "done with conflicts", so errors are reported as `UsageError`.
pub fn run() -> anyhow::Result<Outcome> {
    match Cli::try_parse() {
        Ok(_cli) => Ok(Outcome::Clean),
        Err(e) => {
            let display_only = matches!(
                e.kind(),
                ErrorKind::DisplayHelp
                    | ErrorKind::DisplayVersion
                    | ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand
            );
            // Best effort: nothing useful can be done if the stream is closed.
            let _ = e.print();
            if display_only {
                Ok(Outcome::Clean)
            } else {
                Err(UsageError.into())
            }
        }
    }
}
