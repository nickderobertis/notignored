//! ty (`# ty: ignore`) suppression parsing.
//!
//! Forms understood, each verified against the pinned ty by
//! `tests/e2e/python_types_parity.rs`:
//!
//! | Source | Scope | Rules |
//! | --- | --- | --- |
//! | `f(x)  # ty: ignore` | line | *(blanket)* |
//! | `f(x)  # ty: ignore[invalid-argument-type]` | line | that rule |
//! | `# ty: ignore` above all code | file | *(blanket)* |
//! | `# ty: ignore` on its own line in the body | next-line | *(blanket)* |
//!
//! Where the comment sits is the whole of ty's scoping rule, and it is the one
//! thing a line-only reading would get wrong:
//!
//! * **Trailing code** — the directive covers that line.
//! * **On its own line above all code** — ty reads it as a file-wide exemption.
//! * **On its own line anywhere else** — it covers the line below.
//!
//! ty honours a directive anywhere in the comment (`# noqa: F401  # ty: ignore`
//! suppresses), so the reported span is the inner `#` that opened it, and any
//! further `# …` is the [`reason`](crate::model::IgnoreDirective::reason).
//!
//! ty also honours mypy's `# type: ignore`, which is reported once, as mypy's;
//! see the README's supported-tools table.

use crate::comments::Comment;
use crate::model::{IgnoreDirective, Scope, Suppressed, Tool};
use crate::source::{Language, SourceFile};
use crate::tools::python;
use crate::tools::ToolParser;

/// Parses ty's `# ty: ignore` family out of Python sources.
#[derive(Debug, Clone, Copy, Default)]
pub struct TyParser;

impl ToolParser for TyParser {
    fn tool(&self) -> Tool {
        Tool::Ty
    }

    fn applies_to(&self, file: &SourceFile) -> bool {
        file.language() == Language::Python
    }

    fn parse(&self, file: &SourceFile) -> Vec<IgnoreDirective> {
        let mut out = Vec::new();
        for comment in file.comments() {
            // One suppression per comment: a second `# ty: ignore` on the same
            // line silences nothing the first did not.
            let Some((segment, rest)) = python::segments(comment)
                .into_iter()
                .find_map(|segment| directive_body(segment.after_hash).map(|rest| (segment, rest)))
            else {
                continue;
            };
            let (rules, reason) = python::rules_and_reason(rest);
            let scope = scope_of(file, comment);
            out.push(IgnoreDirective {
                tool: Tool::Ty,
                scope,
                rules,
                reason,
                path: file.display_path().to_string(),
                line: comment.line,
                end_line: comment.end_line,
                column: segment.column,
                raw: segment.raw.to_string(),
                suppressed: suppressed_range(scope, comment.line),
                change: None,
            });
        }
        out
    }
}

/// How far a directive in `comment` reaches, per ty's placement rules.
fn scope_of(file: &SourceFile, comment: &Comment) -> Scope {
    if !comment.leading {
        Scope::Line
    } else if in_file_header(file, comment) {
        Scope::File
    } else {
        Scope::NextLine
    }
}

/// True when nothing but blank lines and whole-line comments precede `comment`.
///
/// ty promotes a directive in that header to a whole-file exemption, so telling
/// the header apart from the body is the difference between reporting one line
/// and reporting a module.
fn in_file_header(file: &SourceFile, comment: &Comment) -> bool {
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

fn suppressed_range(scope: Scope, line: u32) -> Suppressed {
    match scope {
        Scope::File => Suppressed {
            start_line: 1,
            end_line: None,
        },
        // ty skips blank lines before applying an own-line directive; the line
        // below is the best-effort answer the record promises.
        Scope::NextLine => Suppressed {
            start_line: line.saturating_add(1),
            end_line: Some(line.saturating_add(1)),
        },
        _ => Suppressed {
            start_line: line,
            end_line: Some(line),
        },
    }
}

/// True when the text after a `#` opens a ty directive.
///
/// The shared segment scan uses this to bound the run before it; see
/// `src/tools/python.rs::segments`.
pub(super) fn opens_directive(after_hash: &str) -> bool {
    directive_body(after_hash).is_some()
}

/// The text after `ty: ignore`, or `None` when this run is something else.
fn directive_body(after_hash: &str) -> Option<&str> {
    let after_prefix = after_hash.trim_start().strip_prefix("ty:")?;
    python::strip_keyword(after_prefix.trim_start(), "ignore")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(source: &str) -> Vec<IgnoreDirective> {
        let file = SourceFile::new("src/app.py", source.to_string());
        TyParser.parse(&file)
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
    fn the_parser_claims_python_files_only() {
        assert_eq!(TyParser.tool(), Tool::Ty);
        assert!(TyParser.applies_to(&SourceFile::new("a.py", String::new())));
        assert!(!TyParser.applies_to(&SourceFile::new("a.js", String::new())));
    }

    #[test]
    fn a_trailing_bare_ignore_covers_its_own_line() {
        let directive = only("import os\nf(1)  # ty: ignore\n");
        assert_eq!(directive.tool, Tool::Ty);
        assert_eq!(directive.scope, Scope::Line);
        assert!(directive.rules.is_empty());
        assert_eq!(directive.reason, None);
        assert_eq!(directive.path, "src/app.py");
        assert_eq!(
            (directive.line, directive.end_line, directive.column),
            (2, 2, 7)
        );
        assert_eq!(directive.raw, "# ty: ignore");
        assert_eq!(
            directive.suppressed,
            Suppressed {
                start_line: 2,
                end_line: Some(2)
            }
        );
    }

    #[test]
    fn rule_names_are_captured_verbatim() {
        assert_eq!(
            only("import os\nf(1)  # ty: ignore[invalid-argument-type]\n").rules,
            vec!["invalid-argument-type"]
        );
        assert_eq!(
            only("import os\nf(1)  # ty: ignore[invalid-argument-type, unresolved-import]\n").rules,
            vec!["invalid-argument-type", "unresolved-import"]
        );
    }

    #[test]
    fn a_trailing_comment_becomes_the_reason() {
        let directive = only(
            "import os\nf(1)  # ty: ignore[invalid-argument-type]  # upstream stub is wrong\n",
        );
        assert_eq!(directive.rules, vec!["invalid-argument-type"]);
        assert_eq!(directive.reason.as_deref(), Some("upstream stub is wrong"));
    }

    #[test]
    fn a_directive_above_all_code_exempts_the_file() {
        let directive = only("#!/usr/bin/env python\n\n# ty: ignore  # generated\nimport os\n");
        assert_eq!(directive.scope, Scope::File);
        assert_eq!(directive.reason.as_deref(), Some("generated"));
        assert_eq!(
            directive.suppressed,
            Suppressed {
                start_line: 1,
                end_line: None
            }
        );
    }

    #[test]
    fn a_directive_on_its_own_line_in_the_body_covers_the_line_below() {
        let directive = only("import os\n\n# ty: ignore[invalid-argument-type]\nf(1)\n");
        assert_eq!(directive.scope, Scope::NextLine);
        assert_eq!(directive.line, 3);
        assert_eq!(
            directive.suppressed,
            Suppressed {
                start_line: 4,
                end_line: Some(4)
            }
        );
    }

    #[test]
    fn a_directive_need_not_open_the_comment() {
        let directive = only("import os\nf(1)  # noqa: F401  # ty: ignore\n");
        assert!(directive.rules.is_empty());
        assert_eq!(directive.raw, "# ty: ignore");
        assert_eq!(directive.column, 21);
        assert_eq!(directive.scope, Scope::Line);
    }

    #[test]
    fn a_record_stops_at_the_next_tools_directive() {
        let directive =
            only("import os\nf(1)  # ty: ignore  # narrowed upstream  # noqa: F401  # unused\n");
        assert_eq!(directive.reason.as_deref(), Some("narrowed upstream"));
        assert_eq!(directive.raw, "# ty: ignore  # narrowed upstream");
    }

    #[test]
    fn the_header_check_reads_only_the_lines_above_the_comment() {
        let file = SourceFile::new("a.py", "\"\"\"Doc.\"\"\"\n# after\n".to_string());
        assert!(!in_file_header(&file, &file.comments()[0]));
        let shebang = SourceFile::new("a.py", "#!/usr/bin/env python\n# after\n".to_string());
        assert!(in_file_header(&shebang, &shebang.comments()[1]));
    }

    #[test]
    fn look_alike_comments_are_not_directives() {
        // ty rejects `ignore-file` itself: "no whitespace after `ignore`".
        assert!(parse("f(1)  # ty: ignore-file\n").is_empty());
        assert!(parse("f(1)  # ty: ignoreX\n").is_empty());
        assert!(parse("f(1)  # TY: IGNORE\n").is_empty());
        assert!(parse("f(1)  # ty: strict\n").is_empty());
        assert!(parse("f(1)  # type: ignore\n").is_empty());
        assert!(parse("f(1)  # security: ignore\n").is_empty());
    }

    #[test]
    fn a_directive_inside_a_string_literal_is_not_reported() {
        assert!(parse("MSG = \"# ty: ignore\"\n").is_empty());
    }

    #[test]
    fn every_directive_in_a_file_is_reported_in_source_order() {
        let found = parse("# ty: ignore\nf(1)  # ty: ignore\ng(2)  # ty: ignore[misc]\n");
        assert_eq!(found.len(), 3);
        assert_eq!(
            found.iter().map(|d| d.line).collect::<Vec<_>>(),
            vec![1, 2, 3]
        );
        assert_eq!(found[0].scope, Scope::File);
    }
}
