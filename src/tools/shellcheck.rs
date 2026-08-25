//! ShellCheck (`# shellcheck disable=…`) suppression parsing.
//!
//! ShellCheck directives are `key=value` pairs in a comment that **precedes** a
//! command. Only `disable` silences anything; `shell=`, `source=`, `enable=` and
//! friends configure the run and are not suppressions.
//!
//! | Source | Scope | Rules |
//! | --- | --- | --- |
//! | `# shellcheck disable=SC2086` (before the first command) | file | `SC2086` |
//! | `# shellcheck disable=SC2086` (after one) | next-line | `SC2086` |
//! | `# shellcheck disable=SC2086,SC2046` | — | both |
//! | `# shellcheck disable=SC2000-SC2100` | — | the range, as written |
//! | `# shellcheck disable=all` | — | *(blanket)* |
//!
//! A code range is recorded as the single token the author wrote
//! (`SC2000-SC2100`), not expanded into the hundred codes it covers: rules are
//! captured verbatim, and an expansion would bury the author's intent under
//! generated noise.
//!
//! Placement decides scope, and it is checked against the real tool:
//!
//! * Before the first **command** — only comments and blank lines above — the
//!   directive applies file-wide.
//! * Otherwise it applies to the command that follows. `suppressed` names the
//!   next line; when that command is compound (a function, an `if`), ShellCheck
//!   silences its whole body, so this range is a floor rather than the full
//!   reach.
//! * On the same line as a command it applies to nothing at all — ShellCheck
//!   reports `SC1126` and drops the directive — so neither does notignored.
//!
//! `# shellcheck` must open the comment (a `# note: shellcheck disable=…` is
//! prose), the keyword is lower-case, and every token after it must be
//! `key=value` until a `#` starts the reason. Trailing prose without that `#` is
//! a parse error to ShellCheck (`SC1072`) that voids the whole directive, so it
//! yields no record here either.

use crate::model::{normalize_reason, IgnoreDirective, Scope, Suppressed, Tool};
use crate::source::{Language, SourceFile};
use crate::tools::ToolParser;

/// Parses ShellCheck's `disable=` directives out of shell scripts.
#[derive(Debug, Clone, Copy, Default)]
pub struct ShellcheckParser;

impl ToolParser for ShellcheckParser {
    fn tool(&self) -> Tool {
        Tool::Shellcheck
    }

    fn applies_to(&self, file: &SourceFile) -> bool {
        file.language() == Language::Shell
    }

    fn parse(&self, file: &SourceFile) -> Vec<IgnoreDirective> {
        let first_command = first_command_line(file);
        let mut out = Vec::new();
        for comment in file.comments() {
            // A directive that trails a command is SC1126: ShellCheck refuses it,
            // so reporting it would invent a suppression that does not exist.
            if !comment.leading {
                continue;
            }
            let Some(body) = comment.raw.strip_prefix('#') else {
                continue;
            };
            let Some((rules, reason)) = directive_body(body) else {
                continue;
            };
            let scope = match first_command {
                Some(command) if comment.line > command => Scope::NextLine,
                _ => Scope::File,
            };
            out.push(IgnoreDirective {
                tool: Tool::Shellcheck,
                scope,
                rules,
                reason,
                path: file.display_path().to_string(),
                line: comment.line,
                end_line: comment.end_line,
                column: comment.column,
                raw: comment.raw.clone(),
                suppressed: suppressed_range(scope, comment.line),
            });
        }
        out
    }
}

fn suppressed_range(scope: Scope, line: u32) -> Suppressed {
    match scope {
        Scope::File => Suppressed {
            start_line: 1,
            end_line: None,
        },
        _ => {
            let next = line.saturating_add(1);
            Suppressed {
                start_line: next,
                end_line: Some(next),
            }
        }
    }
}

/// The first line holding a command, or `None` for a file that is all comments
/// and blank lines.
///
/// A whole-line comment is exactly a comment that leads its line, so the
/// extracted comments answer this without re-scanning for shell syntax.
fn first_command_line(file: &SourceFile) -> Option<u32> {
    let comment_lines: Vec<u32> = file
        .comments()
        .iter()
        .filter(|comment| comment.leading)
        .map(|comment| comment.line)
        .collect();
    file.source()
        .lines()
        .enumerate()
        .map(|(index, text)| {
            (
                u32::try_from(index).unwrap_or(u32::MAX).saturating_add(1),
                text,
            )
        })
        .find(|(line, text)| !text.trim().is_empty() && !comment_lines.contains(line))
        .map(|(line, _)| line)
}

/// True when the text after a `#` opens a ShellCheck suppression.
///
/// The line-below boundary uses this; see [`crate::tools::opens_directive`].
pub(super) fn opens_directive(after_hash: &str) -> bool {
    directive_body(after_hash).is_some()
}

/// The rules and reason of a `shellcheck disable=` comment, or `None` when the
/// comment is something else.
fn directive_body(after_hash: &str) -> Option<(Vec<String>, Option<String>)> {
    let rest = after_hash.trim_start().strip_prefix("shellcheck")?;
    // A word boundary: `# shellcheckish` is prose.
    if !rest.starts_with(char::is_whitespace) {
        return None;
    }
    disabled_rules(rest)
}

/// The rules a `shellcheck` directive disables and its trailing reason, or
/// `None` when the directive disables nothing or ShellCheck would reject it.
fn disabled_rules(directive: &str) -> Option<(Vec<String>, Option<String>)> {
    let mut rules = Vec::new();
    let mut blanket = false;
    let mut disables = false;
    let mut reason = None;
    let mut rest = directive.trim_start();

    while !rest.is_empty() {
        if let Some(comment) = rest.strip_prefix('#') {
            reason = normalize_reason(comment);
            break;
        }
        let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
        let (token, remainder) = rest.split_at(end);
        // Every token is `key=value`; anything else is SC1072, which voids the
        // whole directive rather than just the token.
        let (key, value) = token.split_once('=')?;
        if key == "disable" {
            disables = true;
            for code in value.split(',').filter(|code| !code.is_empty()) {
                if code == "all" {
                    blanket = true;
                } else {
                    rules.push(code.to_string());
                }
            }
        }
        rest = remainder.trim_start();
    }

    if !disables || (rules.is_empty() && !blanket) {
        // `disable=` with no codes disables nothing, and a directive with no
        // `disable` key at all is configuration, not a suppression.
        return None;
    }
    // `disable=all` is a blanket suppression, which the record spells as no
    // rules at all.
    Some((if blanket { Vec::new() } else { rules }, reason))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(source: &str) -> Vec<IgnoreDirective> {
        let file = SourceFile::new("scripts/deploy.sh", source.to_string());
        ShellcheckParser.parse(&file)
    }

    fn only(source: &str) -> IgnoreDirective {
        let mut found = parse(source);
        assert_eq!(
            found.len(),
            1,
            "expected one directive in {source:?}: {found:#?}"
        );
        found.remove(0)
    }

    #[test]
    fn the_parser_claims_shell_files_only() {
        assert_eq!(ShellcheckParser.tool(), Tool::Shellcheck);
        assert!(ShellcheckParser.applies_to(&SourceFile::new("a.sh", String::new())));
        assert!(!ShellcheckParser.applies_to(&SourceFile::new("a.py", String::new())));
    }

    #[test]
    fn a_directive_before_the_first_command_covers_the_file() {
        let directive = only("#!/bin/bash\n# shellcheck disable=SC2086\necho $1\n");
        assert_eq!(directive.tool, Tool::Shellcheck);
        assert_eq!(directive.scope, Scope::File);
        assert_eq!(directive.rules, vec!["SC2086"]);
        assert_eq!(directive.reason, None);
        assert_eq!(directive.path, "scripts/deploy.sh");
        assert_eq!(
            (directive.line, directive.end_line, directive.column),
            (2, 2, 1)
        );
        assert_eq!(directive.raw, "# shellcheck disable=SC2086");
        assert_eq!(
            directive.suppressed,
            Suppressed {
                start_line: 1,
                end_line: None
            }
        );
    }

    #[test]
    fn a_directive_after_a_command_covers_the_next_line() {
        let directive = only("#!/bin/bash\necho hi\n# shellcheck disable=SC2086\necho $1\n");
        assert_eq!(directive.scope, Scope::NextLine);
        assert_eq!(
            directive.suppressed,
            Suppressed {
                start_line: 4,
                end_line: Some(4)
            }
        );
    }

    #[test]
    fn an_assignment_counts_as_the_first_command() {
        assert_eq!(
            only("#!/bin/bash\nX=1\n# shellcheck disable=SC2086\necho $1\n").scope,
            Scope::NextLine
        );
    }

    #[test]
    fn blank_lines_and_comments_do_not_start_the_script() {
        assert_eq!(
            only("#!/bin/bash\n\n# a note\n\n# shellcheck disable=SC2086\necho $1\n").scope,
            Scope::File
        );
    }

    #[test]
    fn several_codes_split_on_commas() {
        assert_eq!(
            only("# shellcheck disable=SC2086,SC2046\necho $1\n").rules,
            vec!["SC2086", "SC2046"]
        );
    }

    #[test]
    fn a_code_range_is_recorded_as_the_token_the_author_wrote() {
        assert_eq!(
            only("# shellcheck disable=SC2000-SC2100\necho $1\n").rules,
            vec!["SC2000-SC2100"]
        );
    }

    #[test]
    fn disable_all_is_a_blanket_suppression() {
        let directive = only("# shellcheck disable=all\necho $1\n");
        assert!(directive.rules.is_empty());
        assert_eq!(directive.scope, Scope::File);
    }

    #[test]
    fn a_trailing_comment_becomes_the_reason() {
        let directive =
            only("#!/bin/bash\n# shellcheck disable=SC2086  #   word   splitting is wanted \n");
        assert_eq!(directive.rules, vec!["SC2086"]);
        assert_eq!(
            directive.reason.as_deref(),
            Some("word splitting is wanted")
        );
        // `raw` is the directive exactly as written, trailing space included.
        assert_eq!(
            directive.raw,
            "# shellcheck disable=SC2086  #   word   splitting is wanted "
        );
    }

    #[test]
    fn several_keys_on_one_line_are_merged_into_one_record() {
        let directive =
            only("# shellcheck shell=bash disable=SC2086 disable=SC2116 # both\necho $1\n");
        assert_eq!(directive.rules, vec!["SC2086", "SC2116"]);
        assert_eq!(directive.reason.as_deref(), Some("both"));
    }

    #[test]
    fn directives_that_shellcheck_rejects_are_not_reported() {
        // Trailing prose without a `#` is SC1072: the directive is voided.
        assert!(
            parse("# shellcheck disable=SC2086 word splitting is wanted\necho $1\n").is_empty()
        );
        // A directive that trails a command is SC1126.
        assert!(parse("#!/bin/bash\necho hi\necho $1 # shellcheck disable=SC2086\n").is_empty());
        // The keyword is lower-case and must open the comment.
        assert!(parse("# ShellCheck disable=SC2086\necho $1\n").is_empty());
        assert!(parse("# note: shellcheck disable=SC2086\necho $1\n").is_empty());
        assert!(parse("# shellcheckdisable=SC2086\necho $1\n").is_empty());
    }

    #[test]
    fn directives_that_disable_nothing_are_not_suppressions() {
        assert!(parse("# shellcheck shell=bash\necho $1\n").is_empty());
        assert!(parse("# shellcheck source=./lib.sh\n. ./lib.sh\n").is_empty());
        assert!(parse("# shellcheck disable=\necho $1\n").is_empty());
    }

    #[test]
    fn a_directive_inside_a_string_is_not_a_comment() {
        assert!(parse("#!/bin/bash\necho '# shellcheck disable=SC2086'\n").is_empty());
    }

    #[test]
    fn every_directive_in_a_file_is_reported_in_source_order() {
        let found = parse(concat!(
            "#!/bin/bash\n",
            "# shellcheck disable=SC2086\n",
            "echo $1\n",
            "# shellcheck disable=SC2046  # command substitution is intended\n",
            "echo $(ls)\n",
        ));
        assert_eq!(found.len(), 2);
        assert_eq!(found.iter().map(|d| d.line).collect::<Vec<_>>(), vec![2, 4]);
        assert_eq!(found[0].scope, Scope::File);
        assert_eq!(found[1].scope, Scope::NextLine);
    }

    #[test]
    fn a_script_with_no_commands_reports_every_directive_at_file_scope() {
        let directive = only("# shellcheck disable=SC2086\n");
        assert_eq!(directive.scope, Scope::File);
    }
}
