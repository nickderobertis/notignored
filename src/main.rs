//! Thin process shell: parse arguments, run the command, propagate its code.

use std::io;
use std::process::ExitCode;

use clap::Parser;

use notignored::cli::{self, Cli};

fn main() -> ExitCode {
    let cli = Cli::parse();
    // Write through anstream's `AutoStream` so the ANSI the human report emits
    // renders everywhere, not just on Unix: on a *legacy* Windows console (no
    // virtual-terminal processing) raw ANSI prints as `←[31m` garbage, and
    // `AutoStream` enables VT when it can and otherwise translates the SGR codes
    // into Win32 console attribute calls. `--color` has already been resolved,
    // so hand the stream that concrete answer rather than letting it re-detect —
    // `Never` is then a no-op strip over already-plain text, which keeps the
    // `json` and `markdown` bytes exactly what the renderer wrote.
    let choice = if cli.color_enabled() {
        anstream::ColorChoice::Always
    } else {
        anstream::ColorChoice::Never
    };
    let mut stdout = anstream::AutoStream::new(io::stdout().lock(), choice);
    let mut stderr = anstream::AutoStream::new(io::stderr().lock(), choice);
    let code = cli::run(&cli, &mut stdout, &mut stderr);
    ExitCode::from(code)
}
