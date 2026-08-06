//! A real checker, run from a root that is spelled through a symlink.
//!
//! Every parity suite compares a path a checker reported against one this suite
//! expects, and [`support::relative_to`](crate::support::relative_to) is the one
//! place that comparison happens. Its symlink half is invisible on the Linux
//! gate, where a scratch directory is the same string however you reach it —
//! but on macOS every temporary directory lives behind `/var -> /private/var`,
//! so a checker that resolves the paths it reports answers with a spelling the
//! test never wrote. Nothing in the suite would notice until CI's macOS leg went
//! red for a reason that reads like a parser bug.
//!
//! So this journey builds that shape deliberately, on any platform with
//! symlinks, and drives the real pinned pyright through it. Pyright is the
//! checker to use: it reports the path the filesystem resolved to, while ESLint
//! reports the spelling it was handed — proof the two directions are a real
//! difference between tools and not a hypothetical.
//!
//! POSIX-only: creating a directory symlink on Windows needs a privilege a CI
//! runner does not grant, and the normalization's Windows half is proven by the
//! hand-written spellings in `support::paths`.
#![cfg(unix)]

use std::path::{Path, PathBuf};

use crate::support::{fixture, pyright_diagnostics, pyright_failures};

/// A scratch directory holding `real/`, a `linked` symlink to it, and a copy of
/// the pyright fixture family's own config and one violating fixture.
///
/// Copied rather than symlinked file by file: the point is a root reached
/// through a link, not a file that is one.
fn symlinked_root() -> (tempfile::TempDir, PathBuf) {
    let scratch = tempfile::tempdir().expect("scratch dir");
    // The temporary directory may itself sit behind a symlink; resolving it
    // first keeps this journey about the link it adds.
    let real = scratch
        .path()
        .canonicalize()
        .expect("resolve the scratch directory")
        .join("real");
    std::fs::create_dir(&real).expect("create the real root");

    let family = fixture("python-types");
    for name in ["pyrightconfig.json", "pyright/violation.py"] {
        let source = family.join(name);
        let target = real.join(Path::new(name).file_name().expect("a file name"));
        std::fs::copy(&source, &target)
            .unwrap_or_else(|error| panic!("copy {}: {error}", source.display()));
    }

    let linked = scratch.path().join("linked");
    std::os::unix::fs::symlink(&real, &linked).expect("symlink the scratch root");
    (scratch, linked)
}

/// The fixture is the family's control: it carries no directive, so pyright
/// flags it and the run has something to name.
#[test]
fn a_checker_run_from_a_symlinked_root_still_names_the_file_relatively() {
    let (_scratch, linked) = symlinked_root();

    // Pyright resolves what it reports, so this is the absolute path the run
    // answers with — the spelling `linked` is not.
    let diagnostics = pyright_diagnostics(&linked, ".", &["violation.py"]);
    assert!(
        !diagnostics.is_empty(),
        "pyright reported nothing for the family's control fixture"
    );
    for (path, _, _) in &diagnostics {
        assert_eq!(
            path, "violation.py",
            "a diagnostic came back spelled as something other than the file asked about"
        );
    }
    assert_eq!(
        pyright_failures(&linked, ".", &["violation.py"]),
        vec!["violation.py"],
        "the reported path did not reduce to the fixture the journey named"
    );
}
