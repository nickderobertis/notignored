//! Rendering a [`Report`] for people and for machines.
//!
//! stdout carries the data — one line per suppression, or the JSON envelope.
//! stderr carries narration: the summary line and any scan errors. Keeping them
//! apart means `notignored | …` pipes findings and nothing else.

use std::collections::BTreeSet;
use std::io::{self, Write};

use anstyle::{AnsiColor, Style};

use crate::model::{Change, IgnoreDirective, Report};

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
/// The `(justification edited)` token a directive carries when the change
/// rewrote its reason and nothing else.
///
/// Green, because it qualifies the finding *downward* — the change silenced
/// nothing new here — and in its plain weight so it does not compete with the
/// summary's bold verdict for the eye.
const EDITED: Style = AnsiColor::Green.on_default();
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

/// How a classified report's suppressions divide between the two things a
/// change can do to one.
///
/// The human summary and the comment heading both name these two numbers, so
/// they are counted in one place rather than in two spellings that could drift.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChangeCounts {
    /// Suppressions the change wrote.
    pub added: usize,
    /// Suppressions that were already there, whose stated justification is what
    /// the change rewrote.
    pub justification_edited: usize,
}

impl ChangeCounts {
    /// How `report` divides, or `None` when it carries no classification at all.
    ///
    /// A run without `--diff` leaves every record unclassified — and so, having
    /// no records, does a `--diff` run that found nothing. Both are `None`: with
    /// nothing found there is nothing to have been added or edited, and every
    /// surface says exactly what it said before there was a word for the
    /// difference.
    ///
    /// A record whose `change` is unset inside a classified report counts as
    /// added, the same reading the action's own count takes of a report an older
    /// build wrote.
    pub fn of(report: &Report) -> Option<ChangeCounts> {
        if !report
            .ignores
            .iter()
            .any(|directive| directive.change.is_some())
        {
            return None;
        }
        let justification_edited = report
            .ignores
            .iter()
            .filter(|directive| directive.change == Some(Change::JustificationEdited))
            .count();
        Some(ChangeCounts {
            added: report.ignores.len() - justification_edited,
            justification_edited,
        })
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
    let tally = match ChangeCounts::of(report) {
        // A classified report names both numbers, zeros included: "your change
        // edited none" is an answer a reviewer wants, and a summary whose shape
        // depends on the numbers is one more thing to read wrong.
        Some(counts) => format!(
            "{}, {}",
            paint(
                &format!("{} added", counts.added),
                count_style(counts.added),
                color
            ),
            paint(
                &plural_edited(counts.justification_edited),
                count_style(counts.justification_edited),
                color
            ),
        ),
        None => paint(
            &plural(report.ignores.len(), "ignore", "ignores"),
            found,
            color,
        ),
    };
    writeln!(
        err,
        "{} {tally} {} {}",
        paint("notignored:", LABEL, color),
        paint("in", LABEL, color),
        paint(&plural(files.len(), "file", "files"), found, color),
    )?;
    Ok(())
}

/// The styling one count of the summary carries: the all-clear when it is zero,
/// the found colour when it is not.
fn count_style(count: usize) -> Style {
    if count == 0 {
        CLEAN
    } else {
        FOUND
    }
}

/// `1 justification edited` / `3 justifications edited` — the noun pluralizes,
/// never the shorthand: a summary that says only "edited" leaves the reader to
/// guess whether the silenced code moved.
fn plural_edited(count: usize) -> String {
    format!(
        "{} edited",
        plural(count, "justification", "justifications")
    )
}

/// One suppression as `path:line:column tool rules (scope) -- reason`.
///
/// A blanket suppression renders its rules as `*`; a directive with no stated
/// reason drops the `-- …` tail. A directive a `--diff` run classified as a
/// rewritten justification carries one more token, right after the scope: the
/// change silenced nothing new there, and the line has to say so before the
/// reason it is about to quote.
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
    if directive.change == Some(Change::JustificationEdited) {
        line.push(' ');
        line.push_str(&paint("(justification edited)", EDITED, color));
    }
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
            change: None,
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

    /// The same directive as a `--diff` run classified it.
    fn classified(change: Change, rules: &[&str], reason: Option<&str>) -> IgnoreDirective {
        IgnoreDirective {
            change: Some(change),
            ..directive(rules, reason, Scope::Line)
        }
    }

    /// The token goes between the scope and the reason, so the line says what
    /// kind of finding it is before it quotes the justification.
    #[test]
    fn a_rewritten_justification_is_named_on_the_line_after_its_scope() {
        let mut report = Report::new();
        report.ignores.push(classified(
            Change::JustificationEdited,
            &["E501"],
            Some("long wrapped URL"),
        ));
        let (out, err) = human(&report);
        assert_eq!(
            out,
            "src/app.py:12:20 ruff E501 (line) (justification edited) -- long wrapped URL\n"
        );
        assert_eq!(
            err,
            "notignored: 0 added, 1 justification edited in 1 file\n"
        );
    }

    /// Everything this node left alone: an added directive renders the line an
    /// unclassified run renders, byte for byte.
    #[test]
    fn an_added_directive_renders_exactly_what_an_unclassified_one_does() {
        let mut added = Report::new();
        added.ignores.push(classified(
            Change::Added,
            &["E501"],
            Some("long wrapped URL"),
        ));
        let mut unclassified = Report::new();
        unclassified
            .ignores
            .push(directive(&["E501"], Some("long wrapped URL"), Scope::Line));
        assert_eq!(human(&added).0, human(&unclassified).0);
        // Only the summary differs, and only because it now has two answers.
        assert_eq!(
            human(&added).1,
            "notignored: 1 added, 0 justifications edited in 1 file\n"
        );
        assert_eq!(human(&unclassified).1, "notignored: 1 ignore in 1 file\n");
    }

    /// Both counts, always, with the noun pluralized on each side.
    #[test]
    fn a_classified_summary_names_both_counts_including_the_zero() {
        let mut report = Report::new();
        report
            .ignores
            .push(classified(Change::Added, &["E501"], Some("why")));
        report
            .ignores
            .push(classified(Change::Added, &["F401"], Some("why")));
        report
            .ignores
            .push(classified(Change::JustificationEdited, &["E501"], None));
        assert_eq!(
            human(&report).1,
            "notignored: 2 added, 1 justification edited in 1 file\n"
        );

        report
            .ignores
            .push(classified(Change::JustificationEdited, &["F401"], None));
        assert_eq!(
            human(&report).1,
            "notignored: 2 added, 2 justifications edited in 1 file\n"
        );
    }

    /// A `--diff` run that found nothing has nothing to divide, so it says what
    /// it always said.
    #[test]
    fn a_report_with_no_findings_carries_no_classification_to_count() {
        assert_eq!(ChangeCounts::of(&Report::new()), None);
        assert_eq!(
            human(&Report::new()).1,
            "notignored: 0 ignores in 0 files\n"
        );

        let mut classified_report = Report::new();
        classified_report
            .ignores
            .push(classified(Change::Added, &["E501"], None));
        assert_eq!(
            ChangeCounts::of(&classified_report),
            Some(ChangeCounts {
                added: 1,
                justification_edited: 0
            })
        );
    }

    /// The marker is a role of its own, not the scope's colour with more words
    /// in it, and each count of the summary is coloured by its own answer.
    #[test]
    fn the_edited_marker_and_both_counts_carry_their_own_colours() {
        let mut report = Report::new();
        report.ignores.push(classified(
            Change::JustificationEdited,
            &["E501"],
            Some("long wrapped URL"),
        ));
        let (out, err) = colored(&report, true);
        assert!(
            out.contains(&format!("{EDITED}(justification edited){EDITED:#}")),
            "{out:?}"
        );
        assert_ne!(
            EDITED.render().to_string(),
            SCOPE.render().to_string(),
            "the marker is indistinguishable from the scope beside it"
        );
        // Nothing added is the all-clear answer to half the question, and one
        // rewritten justification is the found answer to the other half.
        assert!(err.contains(&format!("{CLEAN}0 added{CLEAN:#}")), "{err:?}");
        assert!(
            err.contains(&format!("{FOUND}1 justification edited{FOUND:#}")),
            "{err:?}"
        );
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
        report.ignores.push(classified(
            Change::JustificationEdited,
            &["F401"],
            Some("re-exported on purpose"),
        ));

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
