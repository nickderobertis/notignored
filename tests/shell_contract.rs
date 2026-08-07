//! Shell this repository's shell can actually run.
//!
//! macOS ships **bash 3.2** as `/bin/bash` — Apple has not shipped a GPLv3 bash,
//! and it is what `just`, a `shell: bash` step, and the composite action all
//! resolve there. Bash 4 syntax does not fail loudly on it: `${name^^}` is a
//! *runtime* "bad substitution" on the line that expands it, so a guard silently
//! evaluates false, a loop silently skips every item, and the script exits 0
//! having done nothing.
//!
//! That is exactly how it landed: `scripts/preserved-log.sh` upshifted credential
//! names with `${name^^}`, and on the macOS leg every credential was skipped, so
//! nothing was redacted — a green-looking script quietly not protecting anything.
//! The Linux gate cannot see any of it.
//!
//! So the construct is kept out of the tree by reading the sources, the way
//! `tests/action_contract.rs` reads `action.yml`: a text check that fails the
//! build is the only thing that runs on every platform's behalf.

use std::path::{Path, PathBuf};

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Bash-4-only parameter-expansion operators, as the suffix each one puts
/// immediately before the closing brace of a `${...}` expansion.
const BASH_4_EXPANSIONS: &[(&str, &str)] = &[
    ("^^", "upper-case expansion `${x^^}`"),
    ("^", "upper-case-first expansion `${x^}`"),
    (",,", "lower-case expansion `${x,,}`"),
    (",", "lower-case-first expansion `${x,}`"),
    ("@Q", "quoted-form expansion `${x@Q}`"),
    ("@U", "upper-case expansion `${x@U}`"),
    ("@L", "lower-case expansion `${x@L}`"),
];

/// Bash-4-only commands and test operators, matched as written.
const BASH_4_CONSTRUCTS: &[(&str, &str)] = &[
    ("declare -A", "associative arrays"),
    ("local -A", "associative arrays"),
    ("mapfile", "`mapfile`"),
    ("readarray", "`readarray`"),
    ("[[ -v ", "the `-v` test operator"),
];

/// Every shell script this repository owns, including the composite action's.
fn shell_scripts() -> Vec<PathBuf> {
    fn walk(dir: &Path, found: &mut Vec<PathBuf>) {
        let entries = std::fs::read_dir(dir)
            .unwrap_or_else(|error| panic!("read {}: {error}", dir.display()));
        for entry in entries.filter_map(Result::ok) {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, found);
            } else if path.extension().is_some_and(|suffix| suffix == "sh") {
                found.push(path);
            }
        }
    }
    let mut found = Vec::new();
    walk(&repo_root().join("scripts"), &mut found);
    found.sort();
    assert!(
        !found.is_empty(),
        "no shell scripts found under scripts/ — this check stopped looking at anything"
    );
    found
}

/// The body of every `${...}` expansion in `source`, in order, ignoring comment
/// lines.
///
/// Scanned rather than pattern-matched because the operators are single
/// characters that appear constantly in ordinary shell — `^` anchors a regex and
/// `,` separates a brace expansion. Only the position immediately before an
/// expansion's own closing brace distinguishes them.
///
/// Comments are skipped so a script can still *name* the construct it avoids,
/// which is the most useful place to explain why.
fn expansions(source: &str) -> Vec<&str> {
    let mut found = Vec::new();
    for line in source.lines() {
        if line.trim_start().starts_with('#') {
            continue;
        }
        let mut at = 0;
        while let Some(open) = line[at..].find("${") {
            let start = at + open + 2;
            let Some(offset) = line[start..].find('}') else {
                break;
            };
            found.push(&line[start..start + offset]);
            at = start + offset + 1;
        }
    }
    found
}

#[test]
fn no_script_uses_a_bash_4_only_parameter_expansion() {
    for script in shell_scripts() {
        let source = std::fs::read_to_string(&script)
            .unwrap_or_else(|error| panic!("read {}: {error}", script.display()));
        for body in expansions(&source) {
            for (operator, description) in BASH_4_EXPANSIONS {
                // The longer operators are checked first, so `${x^^}` is reported
                // as `^^` rather than as `^`.
                if body.len() > operator.len() && body.ends_with(operator) {
                    panic!(
                        "{} uses {description} in `${{{body}}}`\n\
                         ACTION: macOS ships bash 3.2, where this is a runtime \
                         'bad substitution' that makes the surrounding guard \
                         silently false. Use `shopt -s nocasematch`, `tr`, or a \
                         case statement instead.",
                        script.display(),
                    );
                }
            }
        }
    }
}

#[test]
fn no_script_uses_a_bash_4_only_construct() {
    for script in shell_scripts() {
        let source = std::fs::read_to_string(&script)
            .unwrap_or_else(|error| panic!("read {}: {error}", script.display()));
        for (line_number, line) in source.lines().enumerate() {
            // The table below names these constructs in its own prose; skip the
            // comments so a script may still explain why it avoids one.
            if line.trim_start().starts_with('#') {
                continue;
            }
            for (construct, description) in BASH_4_CONSTRUCTS {
                assert!(
                    !line.contains(construct),
                    "{}:{} uses {description}, which bash 3.2 does not have\n\
                     ACTION: macOS ships bash 3.2 and every recipe here runs on \
                     it; use a bash 3.2 equivalent.",
                    script.display(),
                    line_number + 1,
                );
            }
        }
    }
}

/// The scanner has to find the operator where it actually appears and leave the
/// ordinary shell that merely contains those characters alone. Written by hand,
/// because the tree is currently clean and a scanner that matched nothing would
/// pass over it either way.
#[test]
fn the_scanner_tells_an_operator_from_ordinary_shell() {
    let flagged = |source: &str| {
        expansions(source).iter().any(|body| {
            BASH_4_EXPANSIONS
                .iter()
                .any(|(operator, _)| body.len() > operator.len() && body.ends_with(operator))
        })
    };

    // The real defect, and its siblings.
    assert!(flagged("[[ ${name^^} =~ $PATTERN ]]"));
    assert!(flagged("printf '%s' \"${value,,}\""));
    assert!(flagged("echo \"${word^}\""));
    assert!(flagged("echo \"${arg@Q}\""));

    // Ordinary shell that happens to hold the same characters.
    assert!(!flagged("grep -Eq '^[a-z]+$' <<<\"${name}\""));
    assert!(!flagged("printf '%s\\n' \"${list[@]}\""));
    assert!(!flagged("echo \"${NX_DAEMON-false}\""));
    assert!(!flagged("path=\"${dir}/${label}.log\""));
    assert!(!flagged("rm -f \"$dir/$label\".[0-9]*.log"));
    // A lone operator is a variable named `^`, not an expansion operator.
    assert!(!flagged("echo \"${^}\""));
    // A comment may name the construct it is explaining the absence of.
    assert!(!flagged("# not `${name^^}`: macOS ships bash 3.2"));
    assert!(flagged("# fine\n[[ ${name^^} = X ]]"));
}
