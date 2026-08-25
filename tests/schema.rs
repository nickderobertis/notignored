//! Locks the serialized wire contract.
//!
//! [`tests/golden/report.json`](../golden/report.json) locks the *data* a scan
//! produces; this file locks the *shape* — field names, field order, the string
//! spellings of every enum, and the envelope. Both must move together, and a
//! shape change bumps `REPORT_VERSION`.

use notignored::{
    Change, IgnoreDirective, Report, ReportError, Scope, Suppressed, Tool, REPORT_VERSION,
};

/// The approved record, verbatim. Do not reformat: field order is part of the
/// contract, and this literal is what proves it.
const APPROVED_RECORD: &str = r##"{
  "tool": "ruff",
  "scope": "line",
  "rules": [
    "E501"
  ],
  "reason": "long wrapped URL",
  "path": "src/app.py",
  "line": 12,
  "end_line": 12,
  "column": 20,
  "raw": "# noqa: E501  # long wrapped URL",
  "suppressed": {
    "start_line": 12,
    "end_line": 12
  }
}"##;

/// The same record as a `--diff` run classifies it. `change` is last, and it is
/// the *only* difference: a field added within version 1 must leave every record
/// a run without `--diff` produces untouched, byte for byte.
const APPROVED_CLASSIFIED_RECORD: &str = r##"{
  "tool": "ruff",
  "scope": "line",
  "rules": [
    "E501"
  ],
  "reason": "long wrapped URL",
  "path": "src/app.py",
  "line": 12,
  "end_line": 12,
  "column": 20,
  "raw": "# noqa: E501  # long wrapped URL",
  "suppressed": {
    "start_line": 12,
    "end_line": 12
  },
  "change": "justification-edited"
}"##;

fn approved_directive() -> IgnoreDirective {
    IgnoreDirective {
        tool: Tool::Ruff,
        scope: Scope::Line,
        rules: vec!["E501".to_string()],
        reason: Some("long wrapped URL".to_string()),
        path: "src/app.py".to_string(),
        line: 12,
        end_line: 12,
        column: 20,
        raw: "# noqa: E501  # long wrapped URL".to_string(),
        suppressed: Suppressed {
            start_line: 12,
            end_line: Some(12),
        },
        change: None,
    }
}

#[test]
fn a_directive_serializes_to_the_approved_record_field_for_field() {
    let json = serde_json::to_string_pretty(&approved_directive()).unwrap();
    assert_eq!(json, APPROVED_RECORD);
}

/// `change` is present only on a run that classified — a `--diff` run. Absent
/// is not a third value: it says the run had no base to answer against, which is
/// what keeps a whole-tree inventory's records exactly what they were.
#[test]
fn an_unclassified_record_omits_change_entirely_rather_than_writing_null() {
    let json = serde_json::to_value(approved_directive()).unwrap();
    assert!(
        !json.as_object().unwrap().contains_key("change"),
        "change must be omitted, never null: {json}"
    );
}

#[test]
fn a_classified_record_serializes_change_last_and_round_trips() {
    let classified = IgnoreDirective {
        change: Some(Change::JustificationEdited),
        ..approved_directive()
    };
    let json = serde_json::to_string_pretty(&classified).unwrap();
    assert_eq!(json, APPROVED_CLASSIFIED_RECORD);

    let parsed: IgnoreDirective = serde_json::from_str(APPROVED_CLASSIFIED_RECORD).unwrap();
    assert_eq!(parsed, classified);

    let added = IgnoreDirective {
        change: Some(Change::Added),
        ..approved_directive()
    };
    assert_eq!(serde_json::to_value(&added).unwrap()["change"], "added");
    let parsed: IgnoreDirective =
        serde_json::from_value(serde_json::to_value(&added).unwrap()).unwrap();
    assert_eq!(parsed, added);
}

/// A field added within version 1 must not move the version: both SDK readers
/// and this crate's own deserializer refuse an envelope claiming one they do not
/// know, so bumping over an *optional* field would break every pinned consumer
/// for something none of them reads.
#[test]
fn a_report_carrying_change_still_reads_as_version_one() {
    let mut report = Report::new();
    report.ignores.push(IgnoreDirective {
        change: Some(Change::Added),
        ..approved_directive()
    });
    let json = serde_json::to_string(&report).unwrap();
    let round_tripped: Report = serde_json::from_str(&json).unwrap();
    assert_eq!(round_tripped, report);
    assert_eq!(round_tripped.version, 1);
    assert_eq!(REPORT_VERSION, 1);
}

#[test]
fn every_change_has_its_documented_spelling() {
    let spellings: Vec<String> = [Change::Added, Change::JustificationEdited]
        .into_iter()
        .map(|change| {
            serde_json::to_value(change)
                .unwrap()
                .as_str()
                .unwrap()
                .to_string()
        })
        .collect();
    assert_eq!(spellings, vec!["added", "justification-edited"]);
    for change in [Change::Added, Change::JustificationEdited] {
        assert_eq!(change.to_string(), change.as_str());
        let parsed: Change = serde_json::from_value(serde_json::json!(change.as_str())).unwrap();
        assert_eq!(parsed, change);
    }
}

#[test]
fn the_approved_record_deserializes_back_into_the_same_directive() {
    let parsed: IgnoreDirective = serde_json::from_str(APPROVED_RECORD).unwrap();
    assert_eq!(parsed, approved_directive());
}

#[test]
fn a_blanket_file_scope_directive_round_trips_with_nulls() {
    let directive = IgnoreDirective {
        rules: vec![],
        reason: None,
        scope: Scope::File,
        suppressed: Suppressed {
            start_line: 1,
            end_line: None,
        },
        ..approved_directive()
    };
    let json = serde_json::to_value(&directive).unwrap();
    assert_eq!(json["rules"], serde_json::json!([]));
    assert!(
        json["reason"].is_null(),
        "reason must be present and null, never omitted"
    );
    assert!(json["suppressed"]["end_line"].is_null());
    assert_eq!(json["scope"], "file");

    let round_tripped: IgnoreDirective = serde_json::from_value(json).unwrap();
    assert_eq!(round_tripped, directive);
}

#[test]
fn the_envelope_carries_exactly_version_ignores_and_errors() {
    let mut report = Report::new();
    report.ignores.push(approved_directive());
    report.errors.push(ReportError {
        path: "src/binary.py".to_string(),
        message: "stream did not contain valid UTF-8".to_string(),
    });

    let json = serde_json::to_string_pretty(&report).unwrap();
    let value: serde_json::Value = serde_json::from_str(&json).unwrap();
    let keys: Vec<&str> = value
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(keys, vec!["version", "ignores", "errors"]);
    assert_eq!(value["version"], REPORT_VERSION);
    assert_eq!(
        value["version"], 1,
        "REPORT_VERSION moved; update the goldens too"
    );

    let error_keys: Vec<&str> = value["errors"][0]
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(error_keys, vec!["path", "message"]);

    let round_tripped: Report = serde_json::from_str(&json).unwrap();
    assert_eq!(round_tripped, report);
}

#[test]
fn every_scope_has_its_documented_spelling() {
    let spellings: Vec<String> = [Scope::Line, Scope::NextLine, Scope::File, Scope::Block]
        .into_iter()
        .map(|scope| {
            serde_json::to_value(scope)
                .unwrap()
                .as_str()
                .unwrap()
                .to_string()
        })
        .collect();
    assert_eq!(spellings, vec!["line", "next-line", "file", "block"]);
}

#[test]
fn every_declared_tool_has_its_documented_spelling() {
    let spellings: Vec<String> = Tool::ALL
        .into_iter()
        .map(|tool| {
            serde_json::to_value(tool)
                .unwrap()
                .as_str()
                .unwrap()
                .to_string()
        })
        .collect();
    assert_eq!(
        spellings,
        vec![
            "eslint",
            "biome",
            "ruff",
            "typescript",
            "mypy",
            "pyright",
            "ty",
            "rust",
            "shellcheck",
            "llmlint",
        ]
    );
    for tool in Tool::ALL {
        let parsed: Tool = serde_json::from_value(serde_json::json!(tool.as_str())).unwrap();
        assert_eq!(parsed, tool);
    }
}
