//! Parity with the real TypeScript compiler.
//!
//! Same shape as [`ruff_parity`](crate::ruff_parity): the pinned `tsc` decides
//! whether a fixture actually type-checks, and notignored has to describe the
//! suppression that made the difference. Neither side is stubbed.
//!
//! `tsc` polices its own directives — an `@ts-expect-error` that suppressed
//! nothing is error TS2578 — so a fixture passing here proves both halves at
//! once: the assignment really was an error, and the directive really silenced it.

use crate::support::{fixture, notignored, parse_report, tsc_passes};

fn parity_dir() -> std::path::PathBuf {
    fixture("typescript-parity")
}

fn report_for(file: &str) -> serde_json::Value {
    let output = notignored(&parity_dir())
        .args([file, "--format", "json"])
        .output()
        .expect("run notignored");
    assert!(output.status.success(), "exit: {:?}", output.status);
    parse_report(&output.stdout)
}

#[test]
fn real_tsc_flags_the_unsuppressed_fixture_and_notignored_reports_nothing() {
    assert!(
        !tsc_passes(&parity_dir().join("violation.ts")),
        "the fixture is supposed to fail the type check; parity proves nothing otherwise"
    );

    let report = report_for("violation.ts");
    assert!(
        report["ignores"].as_array().unwrap().is_empty(),
        "nothing is suppressed here: {report:#}"
    );
}

#[test]
fn an_expect_error_makes_real_tsc_pass_and_notignored_describes_it_exactly() {
    let file = parity_dir().join("suppressed.ts");
    assert!(
        tsc_passes(&file),
        "the `@ts-expect-error` should make tsc pass; the fixture or the pin drifted"
    );

    let report = report_for("suppressed.ts");
    let ignores = report["ignores"].as_array().unwrap();
    assert_eq!(ignores.len(), 1, "{report:#}");

    let directive = &ignores[0];
    assert_eq!(directive["tool"], "typescript");
    assert_eq!(directive["scope"], "next-line");
    assert_eq!(
        directive["rules"],
        serde_json::json!([]),
        "tsc cannot name an error code, so every directive is blanket"
    );
    assert_eq!(directive["reason"], "the vendored SDK ships no types");
    assert_eq!(directive["path"], "suppressed.ts");
    assert_eq!(directive["line"], 1);
    assert_eq!(directive["end_line"], 1);
    assert_eq!(directive["column"], 1);
    assert_eq!(
        directive["raw"],
        "// @ts-expect-error the vendored SDK ships no types"
    );
    assert_eq!(directive["suppressed"]["start_line"], 2);
    assert_eq!(directive["suppressed"]["end_line"], 2);

    // The reported column really is where the directive starts.
    let source = std::fs::read_to_string(&file).unwrap();
    let column = directive["column"].as_u64().unwrap() as usize;
    assert!(
        source.lines().next().unwrap()[column - 1..].starts_with("// @ts-expect-error"),
        "column {column} does not point at the directive"
    );
}

#[test]
fn a_nocheck_makes_real_tsc_pass_and_is_reported_as_file_scope() {
    assert!(
        tsc_passes(&parity_dir().join("nocheck.ts")),
        "the `@ts-nocheck` should exempt the whole file"
    );

    let report = report_for("nocheck.ts");
    let ignores = report["ignores"].as_array().unwrap();
    assert_eq!(ignores.len(), 1, "{report:#}");

    let directive = &ignores[0];
    assert_eq!(directive["scope"], "file");
    assert_eq!(
        directive["reason"],
        "this bundle predates our type definitions"
    );
    assert_eq!(directive["suppressed"]["start_line"], 1);
    assert!(
        directive["suppressed"]["end_line"].is_null(),
        "a file-scope suppression runs to end-of-file"
    );
}

#[test]
fn every_grammar_form_passes_real_tsc_and_is_reported_with_its_span() {
    assert!(
        tsc_passes(&parity_dir().join("grammar.ts")),
        "real tsc rejected a form this crate claims to parse (or found an unused \
         @ts-expect-error); the grammar or the pin drifted"
    );

    let report = report_for("grammar.ts");
    let ignores = report["ignores"].as_array().unwrap();
    let described: Vec<_> = ignores
        .iter()
        .map(|d| {
            (
                d["line"].as_u64().unwrap(),
                d["end_line"].as_u64().unwrap(),
                d["reason"].as_str().unwrap().to_string(),
                d["suppressed"]["start_line"].as_u64().unwrap(),
            )
        })
        .collect();

    assert_eq!(
        described,
        vec![
            // `// @ts-expect-error …`
            (1, 1, "the next-line form".into(), 2),
            // `/* @ts-ignore … */`
            (3, 3, "the single-line block form".into(), 4),
            // `/** @ts-expect-error … */`
            (5, 5, "the JSDoc block form".into(), 6),
            // a block comment whose directive sits on its *last* line, which is
            // the only multi-line form tsc honours
            (7, 8, "on the comment's last line".into(), 9),
        ],
        "{report:#}"
    );

    for directive in ignores {
        assert_eq!(directive["tool"], "typescript");
        assert_eq!(directive["scope"], "next-line");
        assert_eq!(directive["rules"], serde_json::json!([]));
    }
}

#[test]
fn removing_the_suppression_flips_real_tsc_back_to_failing() {
    // The recovery half of the journey: the same source without its directive
    // must fail the type check again, so the pass above is attributable to the
    // directive and not to an assignment that was always fine.
    let dir = tempfile::tempdir().unwrap();
    let stripped = dir.path().join("stripped.ts");
    let source = std::fs::read_to_string(parity_dir().join("suppressed.ts")).unwrap();
    let without_directive = source
        .split_once('\n')
        .map(|(_, code)| code.to_string())
        .expect("a directive line");
    std::fs::write(&stripped, &without_directive).unwrap();

    assert!(
        !tsc_passes(&stripped),
        "stripping the directive must reinstate the type error"
    );

    let output = notignored(dir.path())
        .args(["--format", "json"])
        .output()
        .expect("run notignored");
    assert!(parse_report(&output.stdout)["ignores"]
        .as_array()
        .unwrap()
        .is_empty());
}
