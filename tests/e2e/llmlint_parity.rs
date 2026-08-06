//! Agreement with the real llmlint.
//!
//! llmlint has no "does this file pass" answer to compare against — its judge
//! tier is a paid model call, and this suite never makes one. What it does have
//! is `check-ignores`: a deterministic, model-free validator of exactly the
//! directives this parser reads. So the parity claim here is agreement on the
//! directive set — llmlint validates a fixture clean and notignored reports the
//! same directives; llmlint rejects one and notignored reports the same file,
//! line, and rule with the same defect visible in the record.

use crate::support::{fixture, llmlint_check_ignores, notignored, parse_report};

fn parity_dir() -> std::path::PathBuf {
    fixture("llmlint-parity")
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
        ],
        "{report:#}"
    );

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
    let (passed, output) = llmlint_check_ignores(&parity_dir().join("invalid"));
    assert!(!passed, "llmlint must reject this fixture:\n{output}");
    assert!(
        output.contains("unclosed.py:1: unclosed ignore-block for rule \"no_debug_prints\""),
        "llmlint no longer words the unclosed-block failure this way:\n{output}"
    );
    assert!(
        output.contains("no_reason.py:1: give a reason"),
        "llmlint no longer words the missing-reason failure this way:\n{output}"
    );

    // A report carrying errors exits 2; the directives are still all reported.
    let report = report_for("invalid", 2);
    let found = report["ignores"].as_array().unwrap();
    assert_eq!(found.len(), 2, "{report:#}");

    let no_reason = &found[0];
    assert_eq!(no_reason["path"], "no_reason.py");
    assert_eq!(no_reason["line"], 1);
    assert_eq!(no_reason["scope"], "line");
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

#[test]
fn a_directive_inside_a_string_literal_is_reported_by_llmlint_but_not_by_notignored() {
    let (passed, output) = llmlint_check_ignores(&parity_dir().join("string-literal"));
    assert!(
        !passed && output.contains("decoy.rs:5"),
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
