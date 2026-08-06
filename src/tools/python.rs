//! Comment grammar shared by the Python type-checker parsers.
//!
//! mypy, pyright, and ty all hang their directives off a `#` comment, all spell
//! their rule lists `[code, code]`, and all leave any further `# …` on the line
//! as free prose. Keeping that grammar in one place is what lets the three
//! modules differ only where the tools themselves differ — which keyword they
//! answer to, whether an embedded directive counts, and how far it reaches.
//!
//! Ruff deliberately does not share this: its `noqa: CODE, CODE` list is
//! colon-delimited and its codes have a shape (`E501`) that these tools' hyphen-
//! and camel-cased rule names do not.

use crate::comments::Comment;
use crate::model::normalize_reason;
use crate::source::SourceFile;

/// One `#`-introduced run inside a comment: a directive candidate.
///
/// A Python comment can hold several (`# type: ignore  # noqa: F401`), and the
/// tools disagree about which ones they honour, so this yields all of them and
/// lets each parser decide.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Segment<'a> {
    /// 1-based column of the `#` that opened this run.
    pub(super) column: u32,
    /// From that `#` to the end of the comment.
    pub(super) raw: &'a str,
    /// Everything after that `#`.
    pub(super) after_hash: &'a str,
}

/// Every `#`-introduced run in `comment`, outermost first.
///
/// Python has no block comments, so a comment never spans lines and the column
/// arithmetic stays on one line.
pub(super) fn segments(comment: &Comment) -> impl Iterator<Item = Segment<'_>> {
    comment
        .raw
        .char_indices()
        .enumerate()
        .filter(|(_, (_, ch))| *ch == '#')
        .map(move |(char_offset, (byte_offset, _))| Segment {
            column: comment
                .column
                .saturating_add(u32::try_from(char_offset).unwrap_or(u32::MAX)),
            raw: &comment.raw[byte_offset..],
            after_hash: &comment.raw[byte_offset + 1..],
        })
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

/// True when nothing but blank lines and whole-line comments precede `comment`.
///
/// mypy and ty both promote a directive in that header to a whole-file
/// exemption, so telling the header apart from the body is the difference
/// between reporting one line and reporting a module.
pub(super) fn in_file_header(file: &SourceFile, comment: &Comment) -> bool {
    file.source()
        .lines()
        .take(usize::try_from(comment.line.saturating_sub(1)).unwrap_or(usize::MAX))
        .enumerate()
        .all(|(index, line)| {
            if line.trim().is_empty() {
                return true;
            }
            let number = u32::try_from(index + 1).unwrap_or(u32::MAX);
            file.comments()
                .iter()
                .any(|earlier| earlier.leading && earlier.line == number)
        })
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
    fn every_hash_in_a_comment_is_a_directive_candidate() {
        let comment = only_comment("x = 1  # type: ignore  # noqa: F401\n");
        assert_eq!(comment.kind, CommentKind::Line);
        let found: Vec<_> = segments(&comment).collect();
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].column, 8);
        assert_eq!(found[0].after_hash, " type: ignore  # noqa: F401");
        assert_eq!(found[1].column, 24);
        assert_eq!(found[1].raw, "# noqa: F401");
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

    #[test]
    fn the_header_ends_at_the_first_line_of_code() {
        let source = "#!/usr/bin/env python\n\n# header\nimport os\n# body\n";
        let file = SourceFile::new("a.py", source.to_string());
        let found = file.comments();
        assert!(in_file_header(&file, &found[0]));
        assert!(in_file_header(&file, &found[1]));
        assert!(!in_file_header(&file, &found[2]));
    }

    #[test]
    fn a_docstring_is_code_so_it_closes_the_header() {
        let file = SourceFile::new("a.py", "\"\"\"Doc.\"\"\"\n# after\n".to_string());
        assert!(!in_file_header(&file, &file.comments()[0]));
    }

    #[test]
    fn the_header_check_reads_only_the_lines_above_the_comment() {
        let file = SourceFile::new("a.py", "x = 1  # here\n# after\n".to_string());
        assert!(in_file_header(&file, &file.comments()[0]));
        assert!(!in_file_header(&file, &file.comments()[1]));
    }
}
