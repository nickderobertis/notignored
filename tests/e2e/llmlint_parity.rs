//! Agreement with the real llmlint.
//!
//! llmlint has no "does this file pass" answer to compare against — its judge
//! tier is a paid model call, and this suite never makes one. What it does have
//! is `check-ignores`: a deterministic, model-free validator of exactly the
//! directives this parser reads. So the parity claim here is agreement on the
//! directive set — llmlint validates a fixture clean and notignored reports the
//! same directives; llmlint rejects one and notignored reports the same file,
//! line, and rule with the same defect visible in the record.
//!
//! `wrapped/` and `refused/` extend that to the reasons themselves. A
//! justification long enough to matter is usually long enough to wrap, and a
//! reason a reviewer reads cut in half is worse than none — so `wrapped/` holds
//! both comment shapes a reason may continue in and llmlint validates every one
//! of them, while `refused/` holds the shapes that must **not** join: llmlint
//! rejects the one whose reason tries to begin below the brackets, and the rest
//! it accepts while notignored stops each reason where the grammar does.

use std::path::Path;

use crate::support::{
    fixture, llmlint_check_ignores, notignored, parse_report, relative_to, split_path_field,
};

fn parity_dir() -> std::path::PathBuf {
    fixture("llmlint-parity")
}

/// The `path:line` locations a `check-ignores` run named, in its own order.
///
/// llmlint reports each finding as `path:line: message`. The path is compared
/// through [`relative_to`] rather than as llmlint spelled it: a substring match
/// on the whole line would pass on any spelling, including one naming a file in
/// another directory entirely.
fn locations(dir: &Path, output: &str) -> Vec<String> {
    output
        .lines()
        .filter_map(|line| {
            let (path, rest) = split_path_field(line.trim())?;
            let number: u32 = rest.split(':').next()?.parse().ok()?;
            Some(format!("{}:{number}", relative_to(dir, path)))
        })
        .collect()
}

/// Run notignored over one fixture sub-tree. Only `invalid/` exits non-zero, so
/// the caller says which code it expects.
fn report_for(sub_tree: &str, expected_code: i32) -> serde_json::Value {
    let output = notignored(&parity_dir().join(sub_tree))
        .args(["--tool", "llmlint", "--format", "json"])
        .output()
        .expect("run notignored");
    assert_eq!(
        output.status.code(),
        Some(expected_code),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    parse_report(&output.stdout)
}

#[test]
fn llmlint_validates_the_clean_fixture_and_notignored_reports_every_directive() {
    let (passed, output) = llmlint_check_ignores(&parity_dir().join("valid"));
    assert!(
        passed,
        "the pinned llmlint rejected the clean fixture; it or the config drifted:\n{output}"
    );

    let report = report_for("valid", 0);
    let found = report["ignores"].as_array().unwrap();
    let described: Vec<(&str, u64, &str, Vec<&str>)> = found
        .iter()
        .map(|directive| {
            (
                directive["path"].as_str().unwrap(),
                directive["line"].as_u64().unwrap(),
                directive["scope"].as_str().unwrap(),
                directive["rules"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|rule| rule.as_str().unwrap())
                    .collect(),
            )
        })
        .collect();
    assert_eq!(
        described,
        vec![
            ("lib.rs", 1, "file", vec!["no_todo_comments"]),
            (
                "lib.rs",
                4,
                "block",
                vec!["no_debug_prints", "errors_are_contextualized"]
            ),
            ("service.py", 1, "file", vec!["errors_are_contextualized"]),
            ("service.py", 4, "block", vec!["no_debug_prints"]),
            ("service.py", 11, "line", vec!["no_todo_comments"]),
            ("service.py", 16, "next-line", vec!["no_debug_prints"]),
        ],
        "{report:#}"
    );

    // Where an `ignore` sits is what its span has to answer for: the trailing
    // one covers the code it shares a line with, the one that has its line to
    // itself covers the code below — which is the only place that code can be.
    assert_eq!(found[4]["suppressed"]["start_line"], 11);
    assert_eq!(found[4]["suppressed"]["end_line"], 11);
    assert_eq!(found[5]["suppressed"]["start_line"], 17);
    assert_eq!(found[5]["suppressed"]["end_line"], 17);

    // llmlint validated every one of these, which means each names configured
    // rules and carries a reason. Both have to be visible in the record.
    let configured = std::fs::read_to_string(parity_dir().join("rules.yml")).unwrap();
    for directive in found {
        for rule in directive["rules"].as_array().unwrap() {
            let rule = rule.as_str().unwrap();
            assert!(
                configured.contains(&format!("name: {rule}")),
                "{rule} is not a configured rule, yet llmlint accepted it"
            );
        }
        assert!(
            directive["reason"].is_string(),
            "llmlint requires a reason, so the record must carry one: {directive}"
        );
    }

    // A closed block spans both of its directives.
    assert_eq!(found[1]["suppressed"]["start_line"], 4);
    assert_eq!(found[1]["suppressed"]["end_line"], 9);
    assert_eq!(found[3]["suppressed"]["start_line"], 4);
    assert_eq!(found[3]["suppressed"]["end_line"], 7);
    // Every rule in `ignore-end[a, b]` closes, none is left dangling.
    assert!(
        report["errors"].as_array().unwrap().is_empty(),
        "{report:#}"
    );
}

#[test]
fn an_unclosed_block_and_a_missing_reason_are_flagged_by_both() {
    let invalid = parity_dir().join("invalid");
    let (passed, output) = llmlint_check_ignores(&invalid);
    assert!(!passed, "llmlint must reject this fixture:\n{output}");
    assert_eq!(
        locations(&invalid, &output),
        vec!["no_reason.py:1", "unclosed.py:1"],
        "llmlint flagged a different set of directives:\n{output}"
    );
    assert!(
        output.contains("unclosed ignore-block for rule \"no_debug_prints\""),
        "llmlint no longer words the unclosed-block failure this way:\n{output}"
    );
    assert!(
        output.contains("give a reason"),
        "llmlint no longer words the missing-reason failure this way:\n{output}"
    );

    // A report carrying errors exits 2; the directives are still all reported.
    let report = report_for("invalid", 2);
    let found = report["ignores"].as_array().unwrap();
    assert_eq!(found.len(), 2, "{report:#}");

    let no_reason = &found[0];
    assert_eq!(no_reason["path"], "no_reason.py");
    assert_eq!(no_reason["line"], 1);
    // Alone on its line, so what it silences is the `TODO` on the line below.
    assert_eq!(no_reason["scope"], "next-line");
    assert_eq!(no_reason["suppressed"]["start_line"], 2);
    assert_eq!(no_reason["suppressed"]["end_line"], 2);
    assert_eq!(no_reason["rules"], serde_json::json!(["no_todo_comments"]));
    assert!(
        no_reason["reason"].is_null(),
        "the missing reason llmlint refuses must read as null: {no_reason}"
    );

    let unclosed = &found[1];
    assert_eq!(unclosed["path"], "unclosed.py");
    assert_eq!(unclosed["line"], 1);
    assert_eq!(unclosed["scope"], "block");
    assert_eq!(unclosed["rules"], serde_json::json!(["no_debug_prints"]));
    assert_eq!(unclosed["suppressed"]["start_line"], 1);
    assert!(
        unclosed["suppressed"]["end_line"].is_null(),
        "an unterminated block has no known end: {unclosed}"
    );

    let errors = report["errors"].as_array().unwrap();
    assert_eq!(errors.len(), 1, "{report:#}");
    assert_eq!(errors[0]["path"], "unclosed.py");
    let message = errors[0]["message"].as_str().unwrap();
    assert!(message.contains("no_debug_prints"), "{message}");
    assert!(message.contains("line 1"), "{message}");
}

/// `(file, line, end_line, reason)` for every directive in a report.
fn reasons(report: &serde_json::Value) -> Vec<(&str, u64, u64, Option<&str>)> {
    report["ignores"]
        .as_array()
        .unwrap()
        .iter()
        .map(|directive| {
            (
                directive["path"].as_str().unwrap(),
                directive["line"].as_u64().unwrap(),
                directive["end_line"].as_u64().unwrap(),
                directive["reason"].as_str(),
            )
        })
        .collect()
}

#[test]
fn llmlint_validates_the_wrapped_fixture_and_notignored_reports_each_reason_whole() {
    let (passed, output) = llmlint_check_ignores(&parity_dir().join("wrapped"));
    assert!(
        passed,
        "the pinned llmlint rejected the wrapped fixture, so its reasons do not \
         begin where this parser reads them:\n{output}"
    );

    let report = report_for("wrapped", 0);
    assert_eq!(
        reasons(&report),
        vec![
            (
                "service.py",
                1,
                3,
                Some(
                    "a thin transport shim: the caller knows which request it made, and a \
                     wrapper added here would only guess at the context it already has"
                )
            ),
            (
                "service.py",
                8,
                10,
                Some(
                    "the trace below is this helper's whole job, and a logger in its place \
                     would need configuration the caller does not have"
                )
            ),
            (
                "tables.rs",
                2,
                3,
                Some(
                    "the vendored grammar tables keep upstream's own markers, and rewriting \
                     one would fork a generated file"
                )
            ),
            (
                "tables.rs",
                6,
                7,
                Some(
                    "the dump below is what this helper is for, and its output is the only \
                     view a maintainer gets of the table"
                )
            ),
        ],
        "{report:#}"
    );

    let found = report["ignores"].as_array().unwrap();
    // What a `next-line` directive covers is the first line that can hold code:
    // past the whole run, so never one of the lines its own reason wrapped onto.
    assert_eq!(found[1]["suppressed"]["start_line"], 11);
    assert_eq!(found[1]["suppressed"]["end_line"], 11);
    assert_eq!(found[3]["suppressed"]["start_line"], 9);
    assert_eq!(found[3]["suppressed"]["end_line"], 9);

    // `raw` is the directive as the source spells it, continuation markers and
    // all — three physical lines here, not one.
    let raw = found[1]["raw"].as_str().unwrap();
    assert_eq!(raw.lines().count(), 3, "{raw:?}");
    assert!(raw.ends_with("# have"), "{raw:?}");
}

#[test]
fn a_reason_stops_where_the_grammar_stops_it() {
    let refused = parity_dir().join("refused");
    let (passed, output) = llmlint_check_ignores(&refused);
    assert!(!passed, "llmlint must reject this fixture:\n{output}");
    assert_eq!(
        locations(&refused, &output),
        vec!["reasonless.py:1"],
        "only the directive whose reason tries to begin below the brackets is \
         invalid to llmlint:\n{output}"
    );
    assert!(
        output.contains("give a reason"),
        "llmlint no longer refuses a reason that starts on the line below:\n{output}"
    );

    // Every other file here is valid llmlint; what the report has to get right
    // is where each reason ends.
    let report = report_for("refused", 0);
    assert_eq!(
        reasons(&report),
        vec![
            // Continues onto line 2, then stops at the bare `#` — a blank
            // comment line is a paragraph break, and the prose under it is
            // commentary rather than justification.
            (
                "boundaries.py",
                1,
                2,
                Some("this shim hands the caller a bare error on purpose")
            ),
            // The line below is a comment indented past this one: a different
            // thought, not the rest of this sentence.
            (
                "boundaries.py",
                9,
                9,
                Some("the trace below is this helper's whole job")
            ),
            // A trailing directive covers the code it shares a line with, so
            // nothing below it continues its reason.
            ("boundaries.py", 11, 11, Some("the marker below is data")),
            // A blank line ends the run.
            (
                "boundaries.py",
                17,
                17,
                Some("printing here is the point of the helper")
            ),
            // A live `# noqa` on the line below is another tool's suppression,
            // and filing it as this one's justification is the inversion this
            // tool exists to prevent.
            (
                "boundaries.py",
                24,
                24,
                Some("the dump below is what this helper is for")
            ),
            // A reason never begins on a continuation line, so this one has
            // none — exactly what llmlint just refused it for.
            ("reasonless.py", 1, 1, None),
        ],
        "{report:#}"
    );
}

#[test]
fn a_directive_inside_a_string_literal_is_reported_by_llmlint_but_not_by_notignored() {
    let literal = parity_dir().join("string-literal");
    let (passed, output) = llmlint_check_ignores(&literal);
    assert!(!passed, "llmlint must reject this fixture:\n{output}");
    assert_eq!(
        locations(&literal, &output),
        vec!["decoy.rs:5"],
        "llmlint scans raw lines, so it should reject the literal's rule name:\n{output}"
    );

    // notignored extracts comments before it looks for directives, so a string
    // literal is never a suppression. This is the one place the two disagree,
    // and it disagrees in the safe direction: no invented record.
    let report = report_for("string-literal", 0);
    assert!(
        report["ignores"].as_array().unwrap().is_empty(),
        "a string literal was reported as a suppression: {report:#}"
    );
}
