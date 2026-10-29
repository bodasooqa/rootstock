//! Binary entry point: maps the outcome of `rootstock::cli::run` to an exit code.

use std::process::ExitCode;

use rootstock::cli::{Outcome, UsageError, run};

fn main() -> ExitCode {
    match run() {
        Ok(Outcome::Clean) => ExitCode::SUCCESS,
        Ok(Outcome::Conflicts(_)) => ExitCode::from(2),
        Err(e) => {
            if e.downcast_ref::<UsageError>().is_none() {
                eprintln!("error: {e:#}");
            }
            ExitCode::from(1)
        }
    }
}
