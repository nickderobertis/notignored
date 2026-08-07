//! `--color` through the real binary: what a terminal, a pipe, and `NO_COLOR`
//! each get.
//!
//! Colour is the one part of the report that depends on *where* the output is
//! going, so it can only be proven by spawning the process with a real
//! environment and a real (non-terminal) stdout. These journeys are also what
//! keeps the promise the other suites rest on: `json` and `markdown` are byte
//! for byte the same whatever `--color` says, so every golden report and every
//! consumer of the envelope is untouched by this flag.

use crate::support::{fixture, notignored};

/// The checked-in tree the format assertions run over.
fn tree() -> std::path::PathBuf {
    fixture("tree")
}

/// Run the binary and return `(stdout, stderr)`, asserting it succeeded.
fn run(args: &[&str], env: &[(&str, &str)]) -> (String, String) {
    let mut command = notignored(&tree());
    command.args(args);
    for (key, value) in env {
        command.env(key, value);
    }
    let output = command.output().expect("run notignored");
    assert!(
        output.status.success(),
        "exit: {:?}: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    (
        String::from_utf8(output.stdout).expect("UTF-8 stdout"),
        String::from_utf8(output.stderr).expect("UTF-8 stderr"),
    )
}

/// Drop every SGR escape, leaving the text a terminal would show.
fn strip_ansi(text: &str) -> String {
    let mut plain = String::new();
    let mut rest = text;
    while let Some(start) = rest.find('\u{1b}') {
        plain.push_str(&rest[..start]);
        let after = &rest[start..];
        let end = after.find('m').expect("an SGR escape ends in `m`");
        rest = &after[end + 1..];
    }
    plain.push_str(rest);
    plain
}

/// A pipe is not a terminal, so the default is the plain report every script,
/// every `| grep`, and every other suite here already depends on.
#[test]
fn the_default_through_a_pipe_is_plain_text() {
    let (stdout, stderr) = run(&[], &[("NO_COLOR", "")]);
    assert!(!stdout.contains('\u{1b}'), "{stdout:?}");
    assert!(!stderr.contains('\u{1b}'), "{stderr:?}");
    assert!(
        stdout.contains("src/app.py:3:12 ruff F401 (line) --"),
        "{stdout}"
    );
}

/// `--color always` is the screenshot/pager escape hatch: it colors through a
/// pipe, and the colored text still reads exactly like the plain one.
#[test]
fn color_always_paints_through_a_pipe_without_changing_the_text() {
    let (plain_out, plain_err) = run(&["--color", "never"], &[("NO_COLOR", "")]);
    let (color_out, color_err) = run(&["--color", "always"], &[("NO_COLOR", "")]);

    assert!(color_out.contains('\u{1b}'), "nothing was colorized");
    assert!(
        color_err.contains('\u{1b}'),
        "the summary was not colorized"
    );
    assert_eq!(strip_ansi(&color_out), plain_out);
    assert_eq!(strip_ansi(&color_err), plain_err);
}

/// `NO_COLOR` is a convention about the *environment*, so an explicit flag has
/// to be able to override it — otherwise a screenshot could never be captured
/// from a shell that sets it.
#[test]
fn no_color_suppresses_auto_but_not_an_explicit_always() {
    let (auto_out, auto_err) = run(&[], &[("NO_COLOR", "1")]);
    assert!(!auto_out.contains('\u{1b}'), "{auto_out:?}");
    assert!(!auto_err.contains('\u{1b}'), "{auto_err:?}");

    let (forced, _) = run(&["--color", "always"], &[("NO_COLOR", "1")]);
    assert!(forced.contains('\u{1b}'), "--color always was vetoed");
}

/// `TERM=dumb` is the other half of the convention: a terminal that cannot
/// render styling asks for none.
#[test]
fn a_dumb_terminal_gets_plain_output() {
    let (stdout, _) = run(&[], &[("NO_COLOR", ""), ("TERM", "dumb")]);
    assert!(!stdout.contains('\u{1b}'), "{stdout:?}");
}

/// The machine formats are contracts, not presentation. `--color` must not move
/// a single byte of either, whatever the terminal or the environment says —
/// this is what lets every golden report stay exactly as it was.
#[test]
fn the_machine_formats_are_byte_identical_whatever_color_says() {
    for format in ["json", "markdown"] {
        let (baseline, _) = run(&["--format", format], &[("NO_COLOR", "1")]);
        assert!(!baseline.is_empty(), "{format} rendered nothing");
        for (args, env) in [
            (
                vec!["--format", format, "--color", "always"],
                vec![("NO_COLOR", "")],
            ),
            (
                vec!["--format", format, "--color", "never"],
                vec![("NO_COLOR", "")],
            ),
            (vec!["--format", format], vec![("NO_COLOR", "")]),
        ] {
            let (actual, _) = run(&args, &env);
            assert_eq!(
                actual, baseline,
                "--format {format} changed under {args:?} / {env:?}"
            );
        }
    }
}
