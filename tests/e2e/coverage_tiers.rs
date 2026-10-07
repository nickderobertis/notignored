//! The coverage floor over the crate's test tiers, executed.
//!
//! The 95% floor is enforced by `scripts/coverage.sh`: each tier records its
//! profiles without reporting, and one report merges every tier's. A mistake in
//! that bookkeeping fails silently in the worst direction — a tier's profiles
//! filed under another, a stale set counted twice, a report over half the suite
//! that still clears the bar on the lines it happened to see — so these journeys
//! run the real script with the real `cargo llvm-cov` over a scratch package
//! small enough to know its coverage exactly: one function only its unit test
//! reaches, one only its integration test reaches.
//!
//! Linux only, like the measurement itself: CI's macOS and Windows legs run the
//! suite without instrumentation (`just test-quick`) and install no
//! `cargo-llvm-cov` to drive.

#![cfg(target_os = "linux")]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use crate::support::{bash_program, repo_root, write};

const MANIFEST: &str = "[package]\nname = \"scratch\"\nversion = \"0.1.0\"\nedition = \"2021\"\n";

/// Two functions of equal size: the unit tier covers the first, the integration
/// tier the second, so either tier alone measures about half the lines and only
/// their union clears the floor.
const LIB: &str = r#"pub fn unit_covered(x: u32) -> u32 {
    let doubled = x * 2;
    let shifted = doubled + 1;
    shifted - x
}

pub fn integration_covered(x: u32) -> u32 {
    let tripled = x * 3;
    let shifted = tripled + 1;
    shifted - x
}

#[cfg(test)]
mod tests {
    #[test]
    fn unit() {
        assert_eq!(super::unit_covered(2), 3);
    }
}
"#;

const INTEGRATION: &str = r#"#[test]
fn integration() {
    assert_eq!(scratch::integration_covered(2), 5);
}
"#;

/// The variables an enclosing instrumented run hands its tests. The scratch run
/// is its own measurement, so none of them may reach it.
fn is_enclosing_run_variable(name: &str) -> bool {
    [
        "LLVM_PROFILE_FILE",
        "CARGO_LLVM_COV",
        "__CARGO_LLVM_COV",
        "NEXTEST",
        "RUSTC_WRAPPER",
        "RUSTC_WORKSPACE_WRAPPER",
        "RUSTFLAGS",
        "RUSTDOCFLAGS",
        "CARGO_ENCODED_RUSTFLAGS",
        "CARGO_TARGET_DIR",
        "CARGO_BUILD_",
    ]
    .iter()
    .any(|prefix| name.starts_with(prefix))
}

fn isolated(mut command: Command, dir: &Path) -> Command {
    command.current_dir(dir);
    for (name, _) in std::env::vars_os() {
        if is_enclosing_run_variable(&name.to_string_lossy()) {
            command.env_remove(name);
        }
    }
    command
}

/// A scratch package with the real script at `scripts/coverage.sh`, so the
/// script resolves the package as its repository root.
fn scratch_package() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = repo_root();
    write(dir.path(), "Cargo.toml", MANIFEST);
    write(dir.path(), "src/lib.rs", LIB);
    write(dir.path(), "tests/integration.rs", INTEGRATION);
    for file in ["scripts/coverage.sh", "rust-toolchain.toml"] {
        let contents = std::fs::read_to_string(root.join(file)).expect("read a repo file");
        write(dir.path(), file, &contents);
    }
    let lockfile = isolated(Command::new("cargo"), dir.path())
        .args(["generate-lockfile", "--offline", "--quiet"])
        .output()
        .expect("run cargo generate-lockfile");
    assert!(
        lockfile.status.success(),
        "{}",
        String::from_utf8_lossy(&lockfile.stderr)
    );
    dir
}

fn coverage(dir: &Path, args: &[&str]) -> Output {
    isolated(Command::new(bash_program()), dir)
        .arg("scripts/coverage.sh")
        .args(args)
        .output()
        .expect("run scripts/coverage.sh")
}

fn profiles_in(dir: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .map(|entry| entry.path())
                .filter(|path| path.extension().is_some_and(|ext| ext == "profraw"))
                .collect()
        })
        .unwrap_or_default()
}

fn text(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

#[test]
fn tiers_combine_into_one_report_that_enforces_the_floor() {
    let package = scratch_package();
    let dir = package.path();
    let store = dir.join("target/coverage-profiles");
    let loose = dir.join("target/llvm-cov-target");

    for (tier, args) in [
        ("unit", &["--lib"][..]),
        ("integration", &["--tests", "-E", "kind(test)"][..]),
    ] {
        let mut command = vec!["tier", tier];
        command.extend_from_slice(args);
        let run = coverage(dir, &command);
        assert!(
            run.status.success(),
            "the {tier} tier failed:\n{}",
            text(&run)
        );
        assert!(
            !profiles_in(&store.join(tier)).is_empty(),
            "the {tier} tier stored no profiles:\n{}",
            text(&run)
        );
        assert!(
            profiles_in(&loose).is_empty(),
            "the {tier} tier left profiles where the next tier would sweep them up"
        );
    }

    // A tier re-run replaces its own set rather than adding to it, and leaves
    // every other tier's alone.
    let before = (
        profiles_in(&store.join("unit")).len(),
        profiles_in(&store.join("integration")),
    );
    let rerun = coverage(dir, &["tier", "unit", "--lib"]);
    assert!(rerun.status.success(), "{}", text(&rerun));
    assert_eq!(
        (
            profiles_in(&store.join("unit")).len(),
            profiles_in(&store.join("integration"))
        ),
        before,
        "re-running the unit tier did not replace exactly its own profiles"
    );

    // One tier alone covers half the code: the floor has to refuse it.
    let half = coverage(dir, &["report", "unit"]);
    assert!(
        !half.status.success(),
        "a report over the unit tier alone cleared the floor:\n{}",
        text(&half)
    );
    assert!(
        text(&half).contains("below 95% lines over the tiers unit"),
        "{}",
        text(&half)
    );

    let union = coverage(dir, &["report", "unit", "integration"]);
    assert!(
        union.status.success(),
        "the union of both tiers did not clear the floor:\n{}",
        text(&union)
    );
    let total = String::from_utf8_lossy(&union.stdout)
        .lines()
        .find(|line| line.starts_with("TOTAL"))
        .map(str::to_string)
        .unwrap_or_else(|| panic!("the report printed no TOTAL:\n{}", text(&union)));
    assert!(total.contains("100.00%"), "{total}");
    assert!(
        profiles_in(&loose).is_empty(),
        "the report left its staged profiles behind for the next tier to adopt"
    );
    assert_eq!(
        profiles_in(&store.join("unit")).len(),
        before.0,
        "the report consumed the stored profiles a cached tier would replay"
    );
}

#[test]
fn a_held_lock_is_waited_on_and_an_abandoned_one_is_taken_over() {
    let package = scratch_package();
    let dir = package.path();
    let run = coverage(dir, &["tier", "unit", "--lib"]);
    assert!(run.status.success(), "{}", text(&run));
    let lock = dir.join("target/coverage-profiles/.lock");

    // Held by a live process — this test's own — the run queues, then gives up
    // naming the holder once its wait runs out.
    write(&lock, "pid", &std::process::id().to_string());
    let held = isolated(Command::new(bash_program()), dir)
        .env("NOTIGNORED_COVERAGE_LOCK_WAIT", "2")
        .args(["scripts/coverage.sh", "tier", "unit", "--lib"])
        .output()
        .expect("run scripts/coverage.sh");
    let said = text(&held);
    assert!(
        !held.status.success()
            && said.contains("waiting for another tier's run")
            && said.contains(&format!("pid {} has held", std::process::id()))
            && said.contains("ACTION:"),
        "a held lock was not waited on and then refused by name:\n{said}"
    );
    assert!(
        lock.is_dir(),
        "the refused run removed a lock it never held"
    );

    // Held by a process that has exited, it is taken over.
    let mut exited = Command::new("true").spawn().expect("spawn true");
    let pid = exited.id();
    exited.wait().expect("wait for true");
    write(&lock, "pid", &pid.to_string());
    let taken = coverage(dir, &["tier", "unit", "--lib"]);
    assert!(
        taken.status.success() && text(&taken).contains(&format!("left by pid {pid}")),
        "an abandoned lock was not taken over:\n{}",
        text(&taken)
    );
    assert!(!lock.exists(), "the run did not release the lock it took");
}

#[test]
fn the_script_refuses_what_it_cannot_measure() {
    let package = tempfile::tempdir().expect("tempdir");
    let dir = package.path();
    let script =
        std::fs::read_to_string(repo_root().join("scripts/coverage.sh")).expect("read script");
    write(dir, "scripts/coverage.sh", &script);
    for (case, args, named) in [
        ("no mode", &[][..], "unknown mode"),
        (
            "a report over nothing",
            &["report"][..],
            "report needs the tiers",
        ),
        (
            "a tier that never ran",
            &["report", "never-ran"][..],
            "never-ran has no recorded profiles",
        ),
        (
            "a project name that is a path",
            &["tier", "../escape", "--lib"][..],
            "'../escape' is not a project name",
        ),
    ] {
        let output = coverage(dir, args);
        let said = text(&output);
        assert!(
            !output.status.success() && said.contains(named) && said.contains("ACTION:"),
            "with {case} the script did not refuse naming the problem and a fix:\n{said}"
        );
    }
    assert!(
        !dir.join("target/coverage-profiles/.lock").exists(),
        "a refused run left a lock behind"
    );
}
