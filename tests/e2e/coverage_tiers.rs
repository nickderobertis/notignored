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

/// A test that holds its tier's run open until the journey releases it, so a
/// second run meets a lock a real run is holding. It marks when it starts and
/// when it is released, in the directory `HOLD_DIR` names.
const HOLD: &str = r#"use std::path::PathBuf;
use std::time::{Duration, Instant};

#[test]
fn holds_the_run_open() {
    let dir = PathBuf::from(std::env::var("HOLD_DIR").expect("HOLD_DIR"));
    std::fs::write(dir.join("started"), "").unwrap();
    let start = Instant::now();
    while !dir.join("release").exists() {
        assert!(start.elapsed() < Duration::from_secs(120), "never released");
        std::thread::sleep(Duration::from_millis(20));
    }
    std::fs::write(dir.join("finished"), "").unwrap();
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
    write(dir.path(), "tests/hold.rs", HOLD);
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
    coverage_with(dir, args, &[])
}

fn coverage_with(dir: &Path, args: &[&str], env: &[(&str, &Path)]) -> Output {
    isolated(Command::new(bash_program()), dir)
        .arg("scripts/coverage.sh")
        .args(args)
        .envs(env.iter().copied())
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
        ("integration", &["--test", "integration"][..]),
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

fn wait_until(what: &str, done: impl Fn() -> bool) {
    let start = std::time::Instant::now();
    while !done() {
        assert!(
            start.elapsed() < std::time::Duration::from_secs(120),
            "timed out waiting until {what}"
        );
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}

/// A tier run held open by the scratch package's `hold` test until `release`
/// is written into `hold_dir`.
fn held_tier(dir: &Path, hold_dir: &Path, stderr: std::process::Stdio) -> std::process::Child {
    std::fs::create_dir_all(hold_dir).expect("create the hold directory");
    let child = isolated(Command::new(bash_program()), dir)
        .env("HOLD_DIR", hold_dir)
        .args(["scripts/coverage.sh", "tier", "holder", "--test", "hold"])
        .stdout(std::process::Stdio::null())
        .stderr(stderr)
        .spawn()
        .expect("start the held tier");
    wait_until("the held tier's test started", || {
        hold_dir.join("started").exists()
    });
    child
}

/// Two tiers at once is what Nx does with a free slot. The second has to queue
/// behind the first rather than sweep up its profiles, give up by name when the
/// first never finishes, and take over the lock of a run killed before it could
/// release it — without adopting the profiles that run left behind.
#[test]
fn a_held_lock_is_waited_on_and_an_abandoned_one_is_taken_over() {
    let package = scratch_package();
    let dir = package.path();
    let store = dir.join("target/coverage-profiles");
    let loose = dir.join("target/llvm-cov-target");
    let lock = store.join(".lock");

    // A live holder: a bounded wait gives up naming it.
    let first = dir.join("hold-1");
    let mut holder = held_tier(dir, &first, std::process::Stdio::null());
    let refused = isolated(Command::new(bash_program()), dir)
        .env("NOTIGNORED_COVERAGE_LOCK_WAIT", "2")
        .args(["scripts/coverage.sh", "tier", "unit", "--lib"])
        .output()
        .expect("run scripts/coverage.sh");
    let said = text(&refused);
    assert!(
        !refused.status.success()
            && said.contains("waiting for another tier's run")
            && said.contains(&format!("pid {} has held", holder.id()))
            && said.contains("ACTION:"),
        "a held lock was not waited on and then refused by name:\n{said}"
    );
    assert!(
        lock.is_dir(),
        "the refused run removed a lock it never held"
    );

    // An unbounded wait queues, and runs once the holder finishes.
    let queued_log = dir.join("queued.log");
    let mut queued = isolated(Command::new(bash_program()), dir)
        .args(["scripts/coverage.sh", "tier", "unit", "--lib"])
        .stdout(std::process::Stdio::null())
        .stderr(std::fs::File::create(&queued_log).expect("create the queued run's log"))
        .spawn()
        .expect("start the queued tier");
    wait_until("the queued run reported that it is waiting", || {
        std::fs::read_to_string(&queued_log)
            .is_ok_and(|log| log.contains("waiting for another tier's run"))
    });
    write(&first, "release", "");
    assert!(holder.wait().expect("wait for the holder").success());
    assert!(
        queued.wait().expect("wait for the queued run").success(),
        "the queued run failed once the lock was free:\n{}",
        std::fs::read_to_string(&queued_log).unwrap_or_default()
    );
    assert!(!profiles_in(&store.join("holder")).is_empty());
    assert!(!profiles_in(&store.join("unit")).is_empty());

    // A holder killed mid-run leaves its lock, and — once its orphaned test
    // exits — profiles that belong to no tier.
    let second = dir.join("hold-2");
    let mut killed = held_tier(dir, &second, std::process::Stdio::null());
    let killed_pid = killed.id();
    let before = profiles_in(&loose).len();
    killed.kill().expect("kill the held tier this test started");
    killed.wait().expect("reap the killed tier");
    write(&second, "release", "");
    wait_until("the killed run's test wrote its profile", || {
        second.join("finished").exists() && profiles_in(&loose).len() > before
    });
    let leftovers: Vec<_> = profiles_in(&loose)
        .iter()
        .filter_map(|path| path.file_name().map(std::ffi::OsStr::to_os_string))
        .collect();
    assert!(lock.is_dir(), "the killed run's lock is already gone");

    let taken = coverage(dir, &["tier", "unit", "--lib"]);
    assert!(
        taken.status.success() && text(&taken).contains(&format!("left by pid {killed_pid}")),
        "an abandoned lock was not taken over:\n{}",
        text(&taken)
    );
    let adopted: Vec<_> = profiles_in(&store.join("unit"))
        .into_iter()
        .filter(|path| {
            path.file_name()
                .is_some_and(|name| leftovers.iter().any(|left| left == name))
        })
        .collect();
    assert!(
        adopted.is_empty(),
        "the unit tier filed the killed run's profiles as its own: {adopted:?}"
    );
    assert!(
        profiles_in(&loose).is_empty(),
        "leftover profiles survived the run"
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

/// The scratch package as an Nx workspace: the unit tier is the root project and
/// the integration tier `tests/`, each `test` declaring its profile directory as
/// its output — the shape this repository's own tiers have — and the unit
/// project's `coverage` combining both. Nx is this repository's own install.
fn scratch_workspace(dir: &Path) {
    write(dir, ".gitignore", "/target\n/.nx\n/node_modules\n");
    write(
        dir,
        "package.json",
        r#"{ "name": "scratch", "private": true }"#,
    );
    write(
        dir,
        "nx.json",
        r#"{
  "namedInputs": { "default": ["{projectRoot}/**/*"] },
  "targetDefaults": {
    "test": { "cache": true, "inputs": ["default", "{workspaceRoot}/src/**/*"] }
  }
}"#,
    );
    write(
        dir,
        "project.json",
        r#"{
  "name": "unit",
  "targets": {
    "test": {
      "command": "bash scripts/coverage.sh tier unit --lib",
      "outputs": ["{workspaceRoot}/target/coverage-profiles/unit"]
    },
    "coverage": {
      "command": "bash scripts/coverage.sh report unit integration",
      "cache": false,
      "dependsOn": ["test", { "projects": ["integration"], "target": "test" }]
    }
  }
}"#,
    );
    write(
        dir,
        "tests/project.json",
        r#"{
  "name": "integration",
  "targets": {
    "test": {
      "command": "bash scripts/coverage.sh tier integration --test integration",
      "outputs": ["{workspaceRoot}/target/coverage-profiles/integration"]
    }
  }
}"#,
    );
    std::os::unix::fs::symlink(repo_root().join("node_modules"), dir.join("node_modules"))
        .expect("link node_modules");
    for args in [
        &["init", "-q"][..],
        &["add", "-A"],
        &["commit", "-qm", "base"],
    ] {
        let git = isolated(Command::new("git"), dir)
            .args([
                "-c",
                "user.email=tester@example.com",
                "-c",
                "user.name=Tester",
            ])
            .args(["-c", "commit.gpgsign=false"])
            .args(args)
            .output()
            .expect("run git");
        assert!(git.status.success(), "{}", text(&git));
    }
}

/// `nx run <target>` in the scratch workspace, with none of the enclosing Nx
/// task's variables.
fn nx_run(dir: &Path, target: &str) -> Output {
    let mut command = isolated(Command::new(dir.join("node_modules/.bin/nx")), dir);
    for (name, _) in std::env::vars_os() {
        if name.to_string_lossy().starts_with("NX_") {
            command.env_remove(name);
        }
    }
    command
        .args(["run", target, "--outputStyle=static"])
        .env("NX_DAEMON", "false")
        .env("NX_USE_LOCAL", "true")
        .output()
        .expect("run nx")
}

/// A cache hit brings a tier's profiles back but not the binaries they count:
/// after a change is made, run and reverted, the binaries on disk were built
/// from the changed sources. The report over the replayed profiles still has to
/// measure the sources as they are.
#[test]
fn a_tier_replayed_from_the_nx_cache_is_measured_against_the_current_sources() {
    let package = scratch_package();
    let dir = package.path();
    scratch_workspace(dir);

    let first = nx_run(dir, "unit:coverage");
    assert!(
        first.status.success() && text(&first).contains("100.00%"),
        "{}",
        text(&first)
    );

    write(
        dir,
        "src/lib.rs",
        &format!("{LIB}\npub fn added_since(x: u32) -> u32 {{\n    x + 7\n}}\n"),
    );
    let changed = nx_run(dir, "unit:test");
    assert!(
        changed.status.success() && !text(&changed).contains("[local cache]"),
        "the changed sources did not rebuild the unit tier:\n{}",
        text(&changed)
    );

    write(dir, "src/lib.rs", LIB);
    let replayed = nx_run(dir, "unit:coverage");
    let said = text(&replayed);
    assert!(
        said.contains("nx run unit:test  [local cache]")
            && said.contains("nx run integration:test  [local cache]"),
        "the reverted sources did not replay both tiers from Nx's cache:\n{said}"
    );
    assert!(
        replayed.status.success() && said.contains("100.00%"),
        "a report over replayed profiles measured binaries built from other sources:\n{said}"
    );
}

/// Profiles are linked into place for the report, and copied when the target
/// directory is on another filesystem than the profile store, where no link can
/// reach.
#[test]
fn profiles_cross_filesystems_when_the_target_directory_is_elsewhere() {
    use std::os::unix::fs::MetadataExt;
    let package = scratch_package();
    let dir = package.path();
    let elsewhere = tempfile::tempdir_in("/dev/shm").expect("a directory on tmpfs");
    let device = |path: &Path| std::fs::metadata(path).expect("stat").dev();
    assert_ne!(
        device(dir),
        device(elsewhere.path()),
        "this journey needs /dev/shm on a filesystem of its own"
    );
    let target = [("CARGO_TARGET_DIR", elsewhere.path())];
    for args in [
        &["tier", "unit", "--lib"][..],
        &["tier", "integration", "--test", "integration"],
    ] {
        let run = coverage_with(dir, args, &target);
        assert!(run.status.success(), "{}", text(&run));
    }
    let report = coverage_with(dir, &["report", "unit", "integration"], &target);
    assert!(
        report.status.success() && text(&report).contains("100.00%"),
        "the report could not stage profiles across filesystems:\n{}",
        text(&report)
    );
    assert!(
        !profiles_in(&dir.join("target/coverage-profiles/unit")).is_empty(),
        "staging moved the stored profiles instead of copying them"
    );
}
