//! Comment grammar shared by the Python `#`-directive parsers.
//!
//! ruff, mypy, pyright, and ty all hang their directives off a `#` comment and
//! all leave any further `# …` on the line as free prose. Keeping that in one
//! place is what lets the four modules differ only where the tools themselves
//! differ — which keyword they answer to, whether an embedded directive counts,
//! and how far one reaches.
//!
//! The load-bearing piece is [`segments`]: one line can carry directives for
//! several tools, and each record's `raw` and `reason` must cover **its own**
//! directive and stop where the next one begins. Without that boundary, a
//! `# type: ignore[import-not-found]  # noqa: F401` would file ruff's live
//! suppression as mypy's stated justification — the exact inversion this tool
//! exists to prevent. Each parser contributes an `opens_directive` recognizer, so
//! the boundary is the union of every grammar the crate understands and no module
//! has to know another's keywords.
//!
//! Rule-list and reason parsing below is shared by the three type checkers only:
//! ruff's `noqa: CODE, CODE` list is colon-delimited and its codes have a shape
//! (`E501`) that those tools' hyphen- and camel-cased rule names do not.

use crate::comments::Comment;
use crate::model::normalize_reason;

/// One `#`-introduced run inside a comment: a directive candidate, bounded by
/// the next run that opens a directive for some tool.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Segment<'a> {
    /// 1-based column of the `#` that opened this run.
    pub(super) column: u32,
    /// From that `#` up to the next tool's directive, trailing space trimmed.
    pub(super) raw: &'a str,
    /// The same run with its opening `#` removed.
    pub(super) after_hash: &'a str,
}

/// Every `#`-introduced run in `comment`, outermost first, each cut short at the
/// next run that opens a directive.
///
/// Python has no block comments, so a comment never spans lines and the column
/// arithmetic stays on one line.
pub(super) fn segments(comment: &Comment) -> Vec<Segment<'_>> {
    // (1-based column offset, byte offset, whether this run opens a directive).
    let hashes: Vec<(usize, usize, bool)> = comment
        .raw
        .char_indices()
        .enumerate()
        .filter(|(_, (_, ch))| *ch == '#')
        .map(|(char_offset, (byte_offset, _))| {
            (
                char_offset,
                byte_offset,
                opens_directive(&comment.raw[byte_offset + 1..]),
            )
        })
        .collect();

    hashes
        .iter()
        .enumerate()
        .map(|(index, &(char_offset, byte_offset, _))| {
            let end = hashes[index + 1..]
                .iter()
                .find(|(_, _, opens)| *opens)
                .map_or(comment.raw.len(), |(_, byte, _)| *byte);
            let raw = comment.raw[byte_offset..end].trim_end();
            Segment {
                column: comment
                    .column
                    .saturating_add(u32::try_from(char_offset).unwrap_or(u32::MAX)),
                raw,
                after_hash: &raw[1..],
            }
        })
        .collect()
}

/// True when the text after a `#` opens a directive for any tool the crate
/// parses — which is what makes it a boundary for the run before it.
///
/// Recognizers mirror their parsers exactly, so nothing this crate declines to
/// report (pyright's `# pyright: basic` mode switch, say) can silently truncate a
/// neighbour's reason.
fn opens_directive(after_hash: &str) -> bool {
    super::ruff::opens_directive(after_hash)
        || super::mypy::opens_directive(after_hash)
        || super::pyright::opens_directive(after_hash)
        || super::ty::opens_directive(after_hash)
}

/// Strip `keyword` from the front of `input`, requiring a word boundary after
/// it so `ignore-file` never reads as `ignore`.
///
/// All three tools match their keywords case-sensitively — real mypy leaves
/// `# TYPE: IGNORE` in force — so this does too.
pub(super) fn strip_keyword<'a>(input: &'a str, keyword: &str) -> Option<&'a str> {
    let rest = input.strip_prefix(keyword)?;
    match rest.chars().next() {
        None => Some(rest),
        Some(ch) if ch.is_alphanumeric() || ch == '_' || ch == '-' => None,
        Some(_) => Some(rest),
    }
}

/// Split the text after a directive keyword into its `[rule, rule]` list and the
/// trailing reason.
///
/// A bracketed list that is empty or never closed degrades to a blanket
/// suppression rather than being dropped: the author plainly meant to silence
/// something, and a review tool that swallows that is worse than one that
/// over-reports.
pub(super) fn rules_and_reason(rest: &str) -> (Vec<String>, Option<String>) {
    let (rules, remainder) = match rest.strip_prefix('[') {
        Some(inside) => match inside.split_once(']') {
            Some((list, after)) => (split_rules(list), after),
            None => (Vec::new(), ""),
        },
        None => (Vec::new(), rest),
    };
    (rules, trailing_reason(remainder))
}

/// The `# …` prose trailing a directive, whitespace collapsed.
///
/// Only a `#` may open it: bare words after the codes are the tool's own syntax,
/// not a justification.
pub(super) fn trailing_reason(remainder: &str) -> Option<String> {
    remainder
        .trim_start()
        .strip_prefix('#')
        .and_then(normalize_reason)
}

/// Split a comma/whitespace-separated rule list, keeping each name verbatim.
///
/// Quotes are stripped because mypy's own `disable-error-code="a, b"` spelling
/// puts them there; the rule name itself never contains one.
pub(super) fn split_rules(list: &str) -> Vec<String> {
    list.split([',', ' ', '\t'])
        .map(|token| token.trim_matches(['"', '\'']))
        .filter(|token| !token.is_empty())
        .map(str::to_string)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::comments::{self, CommentKind};
    use crate::source::Language;

    fn only_comment(source: &str) -> Comment {
        let mut found = comments::extract(source, Language::Python).comments;
        assert_eq!(found.len(), 1, "expected one comment in {source:?}");
        found.remove(0)
    }

    #[test]
    fn a_run_stops_where_the_next_tools_directive_begins() {
        let comment = only_comment("x = 1  # type: ignore  # noqa: F401\n");
        assert_eq!(comment.kind, CommentKind::Line);
        let found = segments(&comment);
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].column, 8);
        assert_eq!(found[0].raw, "# type: ignore");
        assert_eq!(found[0].after_hash, " type: ignore");
        assert_eq!(found[1].column, 24);
        assert_eq!(found[1].raw, "# noqa: F401");
    }

    #[test]
    fn each_run_keeps_the_prose_that_is_only_prose() {
        let comment = only_comment(
            "x = 1  # type: ignore[arg-type]  # no stub  # noqa: F401  # side effects\n",
        );
        let found = segments(&comment);
        assert_eq!(found.len(), 4);
        assert_eq!(found[0].raw, "# type: ignore[arg-type]  # no stub");
        // The run that is pure prose still ends at the next directive.
        assert_eq!(found[1].raw, "# no stub");
        assert_eq!(found[2].raw, "# noqa: F401  # side effects");
        assert_eq!(found[3].raw, "# side effects");
    }

    #[test]
    fn a_lone_directive_runs_to_the_end_of_the_comment() {
        let comment = only_comment("x = 1  # noqa: E501  # long wrapped URL\n");
        let found = segments(&comment);
        assert_eq!(found[0].raw, "# noqa: E501  # long wrapped URL");
    }

    #[test]
    fn every_tools_grammar_counts_as_a_boundary() {
        for directive in [
            "noqa: E501",
            "ruff: noqa",
            "type: ignore",
            "mypy: ignore-errors",
            "mypy: disable-error-code=misc",
            "pyright: ignore",
            "ty: ignore",
        ] {
            let source = format!("x = 1  # noqa: F401  # {directive}\n");
            let comment = only_comment(&source);
            assert_eq!(
                segments(&comment)[0].raw,
                "# noqa: F401",
                "`# {directive}` should have bounded the run before it"
            );
        }
        // Prose, and a mode switch the crate does not report, are not boundaries.
        for prose in ["just a comment", "pyright: basic"] {
            let source = format!("x = 1  # noqa: F401  # {prose}\n");
            let comment = only_comment(&source);
            assert_eq!(
                segments(&comment)[0].raw,
                format!("# noqa: F401  # {prose}")
            );
        }
    }

    #[test]
    fn a_keyword_needs_a_word_boundary_after_it() {
        assert_eq!(strip_keyword("ignore", "ignore"), Some(""));
        assert_eq!(strip_keyword("ignore[a]", "ignore"), Some("[a]"));
        assert_eq!(strip_keyword("ignore  # why", "ignore"), Some("  # why"));
        assert_eq!(strip_keyword("ignore-file", "ignore"), None);
        assert_eq!(strip_keyword("ignored", "ignore"), None);
        assert_eq!(strip_keyword("ignore_all", "ignore"), None);
        assert_eq!(strip_keyword("IGNORE", "ignore"), None);
    }

    #[test]
    fn bracketed_rules_split_on_commas_and_whitespace() {
        assert_eq!(rules_and_reason("[arg-type]").0, vec!["arg-type"]);
        assert_eq!(
            rules_and_reason("[arg-type, index]").0,
            vec!["arg-type", "index"]
        );
        assert_eq!(
            rules_and_reason("[arg-type,index]").0,
            vec!["arg-type", "index"]
        );
        assert_eq!(rules_and_reason("[reportAny]").0, vec!["reportAny"]);
    }

    #[test]
    fn a_missing_or_empty_bracket_list_is_a_blanket_suppression() {
        assert!(rules_and_reason("").0.is_empty());
        assert!(rules_and_reason("[]").0.is_empty());
        assert!(rules_and_reason("[arg-type").0.is_empty());
    }

    #[test]
    fn an_unterminated_bracket_list_swallows_its_reason_too() {
        // There is no directive boundary left to find, so claiming the rest of
        // the line as a justification would invent one.
        assert_eq!(rules_and_reason("[arg-type  # why").1, None);
    }

    #[test]
    fn only_a_hash_opens_a_reason() {
        assert_eq!(
            rules_and_reason("[arg-type]  # upstream stub is wrong").1,
            Some("upstream stub is wrong".to_string())
        );
        assert_eq!(
            rules_and_reason("  # spaced   out ").1,
            Some("spaced out".into())
        );
        assert_eq!(rules_and_reason("[arg-type] because reasons").1, None);
        assert_eq!(rules_and_reason("[arg-type]  #   ").1, None);
    }

    #[test]
    fn quoted_rule_lists_keep_only_the_names() {
        assert_eq!(
            split_rules("\"arg-type, index\""),
            vec!["arg-type", "index"]
        );
        assert_eq!(split_rules(""), Vec::<String>::new());
    }
}
