//! Binary entry point: maps the outcome of `rootstock::cli::run` to an exit code.

use std::process::ExitCode;

use rootstock::cli::{Outcome, run};

fn main() -> ExitCode {
    match run() {
        Ok(Outcome::Clean) => ExitCode::SUCCESS,
        Ok(Outcome::Conflicts(_)) => ExitCode::from(2),
        Err(e) => {
            eprintln!("error: {e:#}");
            ExitCode::from(1)
        }
    }
}
