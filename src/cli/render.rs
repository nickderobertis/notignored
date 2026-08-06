//! Rendering a [`Report`] for people and for machines.
//!
//! stdout carries the data — one line per suppression, or the JSON envelope.
//! stderr carries narration: the summary line and any scan errors. Keeping them
//! apart means `notignored | …` pipes findings and nothing else.

use std::collections::BTreeSet;
use std::io::{self, Write};

use crate::model::{IgnoreDirective, Report};

/// Write the report as one line per suppression, with a summary on `err`.
pub fn render_human(report: &Report, out: &mut dyn Write, err: &mut dyn Write) -> io::Result<()> {
    for directive in &report.ignores {
        writeln!(out, "{}", human_line(directive))?;
    }
    out.flush()?;

    for error in &report.errors {
        writeln!(err, "notignored: error: {}: {}", error.path, error.message)?;
    }
    let files: BTreeSet<&str> = report.ignores.iter().map(|d| d.path.as_str()).collect();
    writeln!(
        err,
        "notignored: {} in {}",
        plural(report.ignores.len(), "ignore", "ignores"),
        plural(files.len(), "file", "files"),
    )?;
    Ok(())
}

/// One suppression as `path:line:column tool rules (scope) -- reason`.
///
/// A blanket suppression renders its rules as `*`; a directive with no stated
/// reason drops the `-- …` tail.
fn human_line(directive: &IgnoreDirective) -> String {
    let rules = if directive.rules.is_empty() {
        "*".to_string()
    } else {
        directive.rules.join(",")
    };
    let mut line = format!(
        "{}:{}:{} {} {} ({})",
        directive.path, directive.line, directive.column, directive.tool, rules, directive.scope
    );
    if let Some(reason) = &directive.reason {
        line.push_str(" -- ");
        line.push_str(reason);
    }
    line
}

fn plural(count: usize, one: &str, many: &str) -> String {
    format!("{count} {}", if count == 1 { one } else { many })
}

/// Write the full report envelope as pretty JSON, newline-terminated.
pub fn render_json(report: &Report, out: &mut dyn Write) -> io::Result<()> {
    serde_json::to_writer_pretty(&mut *out, report)?;
    out.write_all(b"\n")?;
    out.flush()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ReportError, Scope, Suppressed, Tool, REPORT_VERSION};

    fn directive(rules: &[&str], reason: Option<&str>, scope: Scope) -> IgnoreDirective {
        IgnoreDirective {
            tool: Tool::Ruff,
            scope,
            rules: rules.iter().map(|r| r.to_string()).collect(),
            reason: reason.map(str::to_string),
            path: "src/app.py".into(),
            line: 12,
            end_line: 12,
            column: 20,
            raw: "# noqa: E501  # long wrapped URL".into(),
            suppressed: match scope {
                Scope::File => Suppressed {
                    start_line: 1,
                    end_line: None,
                },
                _ => Suppressed {
                    start_line: 12,
                    end_line: Some(12),
                },
            },
        }
    }

    fn human(report: &Report) -> (String, String) {
        let (mut out, mut err) = (Vec::new(), Vec::new());
        render_human(report, &mut out, &mut err).unwrap();
        (
            String::from_utf8(out).unwrap(),
            String::from_utf8(err).unwrap(),
        )
    }

    #[test]
    fn a_coded_directive_renders_its_rules_and_reason() {
        let mut report = Report::new();
        report
            .ignores
            .push(directive(&["E501"], Some("long wrapped URL"), Scope::Line));
        let (out, err) = human(&report);
        assert_eq!(
            out,
            "src/app.py:12:20 ruff E501 (line) -- long wrapped URL\n"
        );
        assert_eq!(err, "notignored: 1 ignore in 1 file\n");
    }

    #[test]
    fn a_blanket_directive_without_a_reason_renders_a_star() {
        let mut report = Report::new();
        report.ignores.push(directive(&[], None, Scope::File));
        let (out, _) = human(&report);
        assert_eq!(out, "src/app.py:12:20 ruff * (file)\n");
    }

    #[test]
    fn several_rules_are_comma_joined() {
        let mut report = Report::new();
        report
            .ignores
            .push(directive(&["E501", "F401"], None, Scope::Line));
        let (out, _) = human(&report);
        assert!(out.contains("ruff E501,F401 (line)"), "{out}");
    }

    #[test]
    fn an_empty_report_says_so_on_stderr_and_nothing_on_stdout() {
        let (out, err) = human(&Report::new());
        assert!(out.is_empty());
        assert_eq!(err, "notignored: 0 ignores in 0 files\n");
    }

    #[test]
    fn scan_errors_are_narrated_on_stderr() {
        let mut report = Report::new();
        report.errors.push(ReportError {
            path: "a.py".into(),
            message: "boom".into(),
        });
        let (out, err) = human(&report);
        assert!(out.is_empty());
        assert!(err.contains("notignored: error: a.py: boom"), "{err}");
    }

    #[test]
    fn the_summary_counts_distinct_files() {
        let mut report = Report::new();
        report.ignores.push(directive(&["E501"], None, Scope::Line));
        let mut second = directive(&["F401"], None, Scope::Line);
        second.path = "src/other.py".into();
        report.ignores.push(second);
        let (_, err) = human(&report);
        assert_eq!(err, "notignored: 2 ignores in 2 files\n");
    }

    #[test]
    fn json_renders_the_documented_envelope() {
        let mut report = Report::new();
        report
            .ignores
            .push(directive(&["E501"], Some("long wrapped URL"), Scope::Line));
        let mut out = Vec::new();
        render_json(&report, &mut out).unwrap();
        let text = String::from_utf8(out).unwrap();
        assert!(text.ends_with("}\n"), "{text}");

        let parsed: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(parsed["version"], REPORT_VERSION);
        assert_eq!(parsed["ignores"][0]["tool"], "ruff");
        assert_eq!(parsed["ignores"][0]["scope"], "line");
        assert_eq!(parsed["ignores"][0]["rules"][0], "E501");
        assert_eq!(parsed["ignores"][0]["suppressed"]["end_line"], 12);
        assert!(parsed["errors"].as_array().unwrap().is_empty());
    }

    #[test]
    fn a_missing_reason_serializes_as_null() {
        let mut report = Report::new();
        report.ignores.push(directive(&[], None, Scope::File));
        let mut out = Vec::new();
        render_json(&report, &mut out).unwrap();
        let parsed: serde_json::Value = serde_json::from_slice(&out).unwrap();
        assert!(parsed["ignores"][0]["reason"].is_null());
        assert!(parsed["ignores"][0]["suppressed"]["end_line"].is_null());
    }
}
