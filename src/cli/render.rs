//! Rendering a [`Report`] for people and for machines.
//!
//! stdout carries the data — one line per suppression, or the JSON envelope.
//! stderr carries narration: the summary line and any scan errors. Keeping them
//! apart means `notignored | …` pipes findings and nothing else.

use std::collections::BTreeSet;
use std::io::{self, Write};

use anstyle::{AnsiColor, Style};

use crate::model::{IgnoreDirective, Report};

// The colour roles of the human line, one per field a reader scans for. Whether
// they are applied is the caller's decision (a plain `bool`) so this module stays
// pure: TTY detection, `NO_COLOR`, and `--color` are resolved in the CLI layer.
//
// Location first (cyan) because it is what a reader jumps to, then the identity
// of the suppression — which checker (magenta) and which rules (yellow) — then
// the reach (blue) and the justification (dim, because it is prose and the
// longest span on the line). A *blanket* suppression's `*` is red: it silences
// every rule the tool has, which is the one thing on the line that deserves to
// stop the eye.
/// `path:line:column`.
const LOCATION: Style = AnsiColor::Cyan.on_default();
/// The tool whose rule is being silenced.
const TOOL: Style = AnsiColor::Magenta.on_default().bold();
/// The named rules.
const RULES: Style = AnsiColor::Yellow.on_default().bold();
/// The `*` of a suppression that names no rules.
const BLANKET: Style = AnsiColor::Red.on_default().bold();
/// How far the directive reaches.
const SCOPE: Style = AnsiColor::Blue.on_default();
/// The stated justification, and the `--` that introduces it.
const REASON: Style = Style::new().dimmed();
/// The `notignored:` label the summary opens with.
const LABEL: Style = Style::new().dimmed();
/// The summary's count when the scan found something.
const FOUND: Style = AnsiColor::Yellow.on_default().bold();
/// The summary's count when it found nothing.
const CLEAN: Style = AnsiColor::Green.on_default().bold();

/// Wrap `text` in `style`'s SGR codes when `color` is on, else return it plain.
///
/// anstyle renders the prefix via `Display` and the reset via the alternate
/// (`{:#}`) form, so a styled span never leaks past its own text.
fn paint(text: &str, style: Style, color: bool) -> String {
    if color {
        format!("{style}{text}{style:#}")
    } else {
        text.to_string()
    }
}

/// Report every scan error on `err`.
///
/// Called for **both** formats: a JSON run pipes stdout to a file, so leaving
/// its errors only inside the envelope would exit 2 with nothing on the
/// terminal to explain why.
pub fn narrate_errors(report: &Report, err: &mut dyn Write) -> io::Result<()> {
    for error in &report.errors {
        writeln!(err, "notignored: error: {}: {}", error.path, error.message)?;
    }
    Ok(())
}

/// Write the report as one line per suppression, in plain text, with a summary
/// on `err`.
///
/// The same output as [`render_human_colored`] with `color` off.
pub fn render_human(report: &Report, out: &mut dyn Write, err: &mut dyn Write) -> io::Result<()> {
    render_human_colored(report, out, err, false)
}

/// Write the report as one line per suppression, with a summary on `err`,
/// ANSI-colorized when `color` is set.
///
/// The decision itself belongs to the caller — see
/// [`ColorChoice::resolve`](crate::cli::ColorChoice::resolve) — so this stays a
/// pure function of the report.
pub fn render_human_colored(
    report: &Report,
    out: &mut dyn Write,
    err: &mut dyn Write,
    color: bool,
) -> io::Result<()> {
    for directive in &report.ignores {
        writeln!(out, "{}", human_line(directive, color))?;
    }
    out.flush()?;

    let files: BTreeSet<&str> = report.ignores.iter().map(|d| d.path.as_str()).collect();
    let found = if report.ignores.is_empty() {
        CLEAN
    } else {
        FOUND
    };
    writeln!(
        err,
        "{} {} {} {}",
        paint("notignored:", LABEL, color),
        paint(
            &plural(report.ignores.len(), "ignore", "ignores"),
            found,
            color
        ),
        paint("in", LABEL, color),
        paint(&plural(files.len(), "file", "files"), found, color),
    )?;
    Ok(())
}

/// One suppression as `path:line:column tool rules (scope) -- reason`.
///
/// A blanket suppression renders its rules as `*`; a directive with no stated
/// reason drops the `-- …` tail.
fn human_line(directive: &IgnoreDirective, color: bool) -> String {
    let rules = if directive.rules.is_empty() {
        paint("*", BLANKET, color)
    } else {
        paint(&directive.rules.join(","), RULES, color)
    };
    let mut line = format!(
        "{} {} {} {}",
        paint(
            &format!("{}:{}:{}", directive.path, directive.line, directive.column),
            LOCATION,
            color
        ),
        paint(&directive.tool.to_string(), TOOL, color),
        rules,
        paint(&format!("({})", directive.scope), SCOPE, color),
    );
    if let Some(reason) = &directive.reason {
        line.push(' ');
        line.push_str(&paint(&format!("-- {reason}"), REASON, color));
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
        colored(report, false)
    }

    fn colored(report: &Report, color: bool) -> (String, String) {
        let (mut out, mut err) = (Vec::new(), Vec::new());
        render_human_colored(report, &mut out, &mut err, color).unwrap();
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
        let mut err = Vec::new();
        narrate_errors(&report, &mut err).unwrap();
        let err = String::from_utf8(err).unwrap();
        assert_eq!(err, "notignored: error: a.py: boom\n");
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

    /// Drop every SGR escape, leaving the text a terminal would show.
    fn strip_ansi(text: &str) -> String {
        let mut plain = String::new();
        let mut rest = text;
        while let Some(start) = rest.find('\u{1b}') {
            plain.push_str(&rest[..start]);
            let after = &rest[start..];
            let end = after.find('m').expect("an SGR escape ends in `m`");
            rest = &after[end + 1..];
        }
        plain.push_str(rest);
        plain
    }

    /// Color is presentation only: the words, spacing, and order a reader sees
    /// have to be the same whether or not the escapes are there, so a script
    /// piping the output and a human watching it never disagree.
    #[test]
    fn coloring_adds_escapes_without_changing_a_single_character() {
        let mut report = Report::new();
        report
            .ignores
            .push(directive(&["E501"], Some("long wrapped URL"), Scope::Line));
        report.ignores.push(directive(&[], None, Scope::File));

        let (plain_out, plain_err) = colored(&report, false);
        let (color_out, color_err) = colored(&report, true);
        assert_ne!(color_out, plain_out, "nothing was colorized: {color_out:?}");
        assert_eq!(strip_ansi(&color_out), plain_out);
        assert_eq!(strip_ansi(&color_err), plain_err);
    }

    /// Each field gets its own role, so a reader can pick the tool out of a
    /// hundred lines without reading any of them.
    #[test]
    fn every_field_of_a_colored_line_carries_its_own_style() {
        let mut report = Report::new();
        report
            .ignores
            .push(directive(&["E501"], Some("long wrapped URL"), Scope::Line));
        let (out, _) = colored(&report, true);
        for (label, style, text) in [
            ("location", LOCATION, "src/app.py:12:20"),
            ("tool", TOOL, "ruff"),
            ("rules", RULES, "E501"),
            ("scope", SCOPE, "(line)"),
            ("reason", REASON, "-- long wrapped URL"),
        ] {
            assert!(
                out.contains(&format!("{style}{text}{style:#}")),
                "the {label} span is not styled as its role: {out:?}"
            );
        }
    }

    /// A directive that names no rules silences everything the tool has, so its
    /// `*` is the one span on the line that gets the alarm colour.
    #[test]
    fn a_blanket_star_is_colored_apart_from_named_rules() {
        let mut report = Report::new();
        report.ignores.push(directive(&[], None, Scope::File));
        let (out, _) = colored(&report, true);
        assert!(out.contains(&format!("{BLANKET}*{BLANKET:#}")), "{out:?}");
        assert_ne!(
            BLANKET.render().to_string(),
            RULES.render().to_string(),
            "a blanket suppression is indistinguishable from a named one"
        );
    }

    /// The summary is the line a reviewer reads first, so its colour has to say
    /// which of the two answers it is before the number is read.
    #[test]
    fn the_summary_count_is_green_when_clean_and_loud_when_not() {
        let (_, clean) = colored(&Report::new(), true);
        assert!(
            clean.contains(&format!("{CLEAN}0 ignores{CLEAN:#}")),
            "{clean:?}"
        );

        let mut report = Report::new();
        report.ignores.push(directive(&["E501"], None, Scope::Line));
        let (_, found) = colored(&report, true);
        assert!(
            found.contains(&format!("{FOUND}1 ignore{FOUND:#}")),
            "{found:?}"
        );
    }

    /// The pre-existing entry point is the plain-text one, unchanged.
    #[test]
    fn render_human_is_the_uncolored_rendering() {
        let mut report = Report::new();
        report
            .ignores
            .push(directive(&["E501"], Some("long wrapped URL"), Scope::Line));
        let (mut out, mut err) = (Vec::new(), Vec::new());
        render_human(&report, &mut out, &mut err).unwrap();
        let (plain_out, plain_err) = colored(&report, false);
        assert_eq!(String::from_utf8(out).unwrap(), plain_out);
        assert_eq!(String::from_utf8(err).unwrap(), plain_err);
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
