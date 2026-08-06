//! Thin process shell: parse arguments, run the command, propagate its code.

use std::io;
use std::process::ExitCode;

use clap::Parser;

use notignored::cli::{self, Cli};

fn main() -> ExitCode {
    let cli = Cli::parse();
    let stdout = io::stdout();
    let stderr = io::stderr();
    let code = cli::run(&cli, &mut stdout.lock(), &mut stderr.lock());
    ExitCode::from(code)
}
