//! The `bash` that can actually run this repository's shell scripts.
//!
//! Spawned by name, `bash` resolves through Windows' own search order, which
//! looks in the system directory *first* and finds `System32\bash.exe` — the WSL
//! launcher. On a runner with no distribution installed that exits without ever
//! running the script, which is not a failure the script could have caused.
//! `just` (through `set shell`) and a workflow's `shell: bash` both use Git for
//! Windows' bash, so resolve the same one and never the system directory's.
//!
//! Included with `#[path]` by every test binary that runs a script, so there is
//! one resolver rather than one per binary.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// Absolute rather than a bare name for the same reason: the answer has to be
/// the one this chose, not whatever the child's PATH would have found.
pub fn bash_program() -> PathBuf {
    let roots = ["ProgramFiles", "ProgramFiles(x86)"]
        .into_iter()
        .filter_map(std::env::var_os)
        .collect::<Vec<_>>();
    let system_root = std::env::var_os("SystemRoot").map(PathBuf::from);
    let name = if cfg!(windows) { "bash.exe" } else { "bash" };
    select_bash(
        name,
        &roots,
        &std::env::var_os("PATH").unwrap_or_default(),
        system_root.as_deref(),
    )
    .unwrap_or_else(|| {
        panic!(
            "no usable {name} found\n\
             ACTION: install bash — `just` runs every recipe through it, so it \
             has to be drivable here too. On Windows that means Git for Windows, \
             not the WSL launcher in the system directory."
        )
    })
}

/// The first `name` found under Git for Windows in one of `program_files`, then
/// along `path` — never inside `system_root`.
pub fn select_bash(
    name: &str,
    program_files: &[OsString],
    path: &OsString,
    system_root: Option<&Path>,
) -> Option<PathBuf> {
    program_files
        .iter()
        .flat_map(|root| {
            let git = PathBuf::from(root).join("Git");
            [git.join("bin"), git.join("usr").join("bin")]
        })
        .chain(std::env::split_paths(path))
        .filter(|dir| !is_system_directory(dir, system_root))
        .map(|dir| dir.join(name))
        .find(|candidate| candidate.is_file())
}

/// Whether `dir` lives under Windows' system root, where `bash.exe` is the WSL
/// launcher rather than a shell.
///
/// Compared case-insensitively: `C:\Windows` and `c:\windows` are one directory,
/// and PATH routinely spells it either way. Compared as a *path* prefix rather
/// than a string one, too — `C:\WindowsApps` is its own directory, and dropping
/// it would take a real PATH entry with it.
fn is_system_directory(dir: &Path, system_root: Option<&Path>) -> bool {
    let Some(system_root) = system_root else {
        return false;
    };
    let normalize = |path: &Path| {
        path.to_string_lossy()
            .to_lowercase()
            .replace('\\', "/")
            .trim_end_matches('/')
            .to_string()
    };
    let (dir, root) = (normalize(dir), normalize(system_root));
    dir == root || dir.starts_with(&format!("{root}/"))
}

/// Which directories a `bash` may come from, proven on the spellings only
/// Windows emits.
///
/// The gate runs on Linux, where nothing named `bash` is ever the WSL launcher,
/// so the rule that keeps it out is written against hand-made Windows paths and
/// a hand-built Windows-shaped tree. Without these the guard is only ever
/// exercised on the one platform that cannot break it, and the next
/// `cross (windows-latest)` run is what finds the regression.
#[cfg(test)]
mod tests {
    use super::{bash_program, is_system_directory, select_bash};
    use std::ffi::OsString;
    use std::path::{Path, PathBuf};

    #[test]
    fn the_windows_system_directory_is_never_a_source_of_bash() {
        let system_root = Path::new(r"C:\Windows");
        for dir in [
            r"C:\Windows\System32",
            r"C:\Windows\system32",
            r"c:\windows\System32\wbem",
            r"C:\Windows",
        ] {
            assert!(
                is_system_directory(Path::new(dir), Some(system_root)),
                "{dir} is inside the system root, where bash.exe is the WSL launcher"
            );
        }
    }

    #[test]
    fn git_for_windows_and_the_tool_directories_still_count() {
        let system_root = Path::new(r"C:\Windows");
        for dir in [
            r"C:\Program Files\Git\bin",
            r"C:\Program Files\Git\usr\bin",
            r"D:\a\_temp\msys64\usr\bin",
            // Not a prefix match on the drive: `C:\WindowsApps` is its own
            // directory, and excluding it would drop a real PATH entry.
            r"C:\WindowsApps",
        ] {
            assert!(
                !is_system_directory(Path::new(dir), Some(system_root)),
                "{dir} is not the system directory and must stay eligible"
            );
        }
    }

    /// Nothing to exclude is not the same as excluding everything: a host that
    /// does not set `SystemRoot` — every Unix one — must keep its whole PATH.
    #[test]
    fn without_a_system_root_no_directory_is_excluded() {
        assert!(!is_system_directory(Path::new("/usr/bin"), None));
    }

    /// A Windows runner's layout, built for real: a `bash.exe` in the system
    /// directory, which Windows' search order would reach first, and another
    /// under Git for Windows.
    struct WindowsTree {
        _root: tempfile::TempDir,
        system_root: PathBuf,
        system32: PathBuf,
        program_files: PathBuf,
        git_bash: PathBuf,
    }

    fn windows_tree() -> WindowsTree {
        let root = tempfile::tempdir().expect("a scratch tree");
        let system_root = root.path().join("Windows");
        let system32 = system_root.join("System32");
        let program_files = root.path().join("Program Files");
        let git_bin = program_files.join("Git").join("bin");
        for dir in [&system32, &git_bin] {
            std::fs::create_dir_all(dir).expect("create a tree directory");
        }
        std::fs::write(system32.join("bash.exe"), "WSL launcher").expect("write WSL bash");
        let git_bash = git_bin.join("bash.exe");
        std::fs::write(&git_bash, "Git bash").expect("write Git bash");
        WindowsTree {
            _root: root,
            system_root,
            system32,
            program_files,
            git_bash,
        }
    }

    fn path_of(dirs: &[&Path]) -> OsString {
        std::env::join_paths(dirs).expect("join PATH")
    }

    #[test]
    fn git_for_windows_wins_over_the_wsl_launcher_ahead_of_it() {
        let tree = windows_tree();
        let chosen = select_bash(
            "bash.exe",
            &[tree.program_files.clone().into_os_string()],
            &path_of(&[&tree.system32]),
            Some(&tree.system_root),
        );
        assert_eq!(chosen, Some(tree.git_bash.clone()));
    }

    /// Git installed somewhere other than Program Files is still found on PATH,
    /// even when the system directory precedes it there.
    #[test]
    fn a_path_bash_behind_the_system_directory_is_chosen_over_it() {
        let tree = windows_tree();
        let git_bin = tree.git_bash.parent().expect("Git's bin directory");
        let chosen = select_bash(
            "bash.exe",
            &[],
            &path_of(&[&tree.system32, git_bin]),
            Some(&tree.system_root),
        );
        assert_eq!(chosen, Some(tree.git_bash.clone()));
    }

    /// With only the WSL launcher available there is no bash to run, and the
    /// resolver says so rather than handing the launcher back.
    #[test]
    fn the_wsl_launcher_alone_is_no_bash_at_all() {
        let tree = windows_tree();
        let chosen = select_bash(
            "bash.exe",
            &[],
            &path_of(&[&tree.system32]),
            Some(&tree.system_root),
        );
        assert_eq!(chosen, None);
    }

    /// The resolver has to answer with a bash that runs, on whichever platform
    /// this is: the journeys that drive `scripts/nx-affected.sh` are not
    /// `cfg(unix)`-gated, so the `cross` legs depend on this too.
    #[test]
    fn the_resolved_bash_runs_a_script() {
        let output = std::process::Command::new(bash_program())
            .args(["-c", "printf ready"])
            .output()
            .expect("run the resolved bash");
        assert!(
            output.status.success() && output.stdout == b"ready",
            "the resolved bash did not run a trivial script:\n{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        );
    }
}
