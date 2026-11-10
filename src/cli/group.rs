//! Which suppressions the `human` and `markdown` reports present as one entry.
//!
//! A change that silences one rule across a codebase writes the same decision
//! many times. Both renderers call [`entries`], so the terminal and the
//! pull-request comment cannot disagree on what counts as the same decision;
//! the report itself, its order and the `json` envelope are untouched.
//!
//! Two suppressions are the same decision when everything a reviewer judges is
//! equal: the tool, the scope, the rules as a set — order and repeats are
//! spelling, case is not — the reason exactly as the parser normalized it, and
//! the change word, so a `--diff` run never folds a rewritten justification
//! into an addition. Where they sit, how their text was spaced and how far they
//! reach are not part of the decision.

use std::collections::{BTreeSet, HashMap};

use crate::model::{IgnoreDirective, Tool};

/// One entry of a rendered report: a suppression, or every suppression
/// identical to it.
///
/// Never empty: the first member is the one whose position the entry takes and
/// whose rule spelling it shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Entry<'a> {
    /// The suppressions this entry stands for, in report order.
    pub(crate) members: Vec<&'a IgnoreDirective>,
}

impl<'a> Entry<'a> {
    /// The member the entry is rendered from.
    pub(crate) fn first(&self) -> &'a IgnoreDirective {
        self.members[0]
    }

    /// Whether this entry stands for more than one suppression, and so lists
    /// its locations under it rather than inline.
    pub(crate) fn is_group(&self) -> bool {
        self.members.len() > 1
    }
}

/// What two suppressions must share to be one entry.
#[derive(PartialEq, Eq, Hash)]
struct Identity<'a> {
    tool: Tool,
    scope: &'static str,
    rules: BTreeSet<&'a str>,
    reason: Option<&'a str>,
    change: Option<&'static str>,
}

impl<'a> Identity<'a> {
    fn of(directive: &'a IgnoreDirective) -> Self {
        Identity {
            tool: directive.tool,
            scope: directive.scope.as_str(),
            rules: directive.rules.iter().map(String::as_str).collect(),
            reason: directive.reason.as_deref(),
            change: directive.change.map(|change| change.as_str()),
        }
    }
}

/// The entries `ignores` renders as.
///
/// With `group` set, identical suppressions share one entry, placed where its
/// first member falls in the report and listing its members in report order.
/// Without it — `--no-group` — every suppression is an entry of its own, which
/// is exactly the output before grouping existed.
pub(crate) fn entries(ignores: &[IgnoreDirective], group: bool) -> Vec<Entry<'_>> {
    if !group {
        return ignores
            .iter()
            .map(|directive| Entry {
                members: vec![directive],
            })
            .collect();
    }
    let mut entries: Vec<Entry<'_>> = Vec::new();
    let mut seen: HashMap<Identity<'_>, usize> = HashMap::new();
    for directive in ignores {
        let index = *seen.entry(Identity::of(directive)).or_insert_with(|| {
            entries.push(Entry {
                members: Vec::new(),
            });
            entries.len() - 1
        });
        entries[index].members.push(directive);
    }
    entries
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Change, Scope, Suppressed};

    fn directive(line: u32, rules: &[&str], reason: Option<&str>) -> IgnoreDirective {
        IgnoreDirective {
            tool: Tool::Ruff,
            scope: Scope::Line,
            rules: rules.iter().map(|rule| rule.to_string()).collect(),
            reason: reason.map(str::to_string),
            path: "src/app.py".into(),
            line,
            end_line: line,
            column: 8,
            raw: format!("# noqa: {}", rules.join(",")),
            suppressed: Suppressed {
                start_line: line,
                end_line: Some(line),
            },
            change: None,
        }
    }

    /// Each entry as the lines of its members.
    fn lines(ignores: &[IgnoreDirective], group: bool) -> Vec<Vec<u32>> {
        entries(ignores, group)
            .iter()
            .map(|entry| entry.members.iter().map(|member| member.line).collect())
            .collect()
    }

    #[test]
    fn rules_group_as_a_set_whatever_their_order_or_repeats() {
        let ignores = [
            directive(1, &["E501", "F401"], Some("why")),
            directive(2, &["F401", "E501"], Some("why")),
            directive(3, &["E501", "F401", "E501"], Some("why")),
        ];
        assert_eq!(lines(&ignores, true), vec![vec![1, 2, 3]]);
    }

    #[test]
    fn a_different_rule_set_is_a_different_decision() {
        let ignores = [
            directive(1, &["E501", "F401"], Some("why")),
            directive(2, &[], Some("why")),
            directive(3, &["F401"], Some("why")),
            directive(4, &["e501", "F401"], Some("why")),
            directive(5, &[], Some("why")),
        ];
        // The two blankets are one decision; nothing else is shared.
        assert_eq!(
            lines(&ignores, true),
            vec![vec![1], vec![2, 5], vec![3], vec![4]]
        );
    }

    #[test]
    fn tool_scope_reason_and_change_each_split_a_group() {
        let base = directive(1, &["E501"], Some("why"));
        let ignores = [
            base.clone(),
            IgnoreDirective {
                tool: Tool::Mypy,
                line: 2,
                ..base.clone()
            },
            IgnoreDirective {
                scope: Scope::File,
                line: 3,
                ..base.clone()
            },
            IgnoreDirective {
                reason: Some("Why".into()),
                line: 4,
                ..base.clone()
            },
            IgnoreDirective {
                reason: None,
                line: 5,
                ..base.clone()
            },
            IgnoreDirective {
                reason: None,
                line: 6,
                ..base.clone()
            },
            IgnoreDirective {
                change: Some(Change::JustificationEdited),
                line: 7,
                ..base.clone()
            },
            IgnoreDirective {
                change: Some(Change::Added),
                line: 8,
                ..base.clone()
            },
        ];
        assert_eq!(
            lines(&ignores, true),
            vec![
                vec![1],
                vec![2],
                vec![3],
                vec![4],
                vec![5, 6],
                vec![7],
                vec![8]
            ]
        );
    }

    #[test]
    fn location_raw_text_and_reach_do_not_split_a_group() {
        let base = directive(1, &["E501"], Some("why"));
        let ignores = [
            base.clone(),
            IgnoreDirective {
                path: "src/other.py".into(),
                line: 40,
                column: 1,
                end_line: 41,
                raw: "#  noqa:E501   #  why".into(),
                suppressed: Suppressed {
                    start_line: 40,
                    end_line: None,
                },
                ..base.clone()
            },
        ];
        assert_eq!(lines(&ignores, true), vec![vec![1, 40]]);
    }

    #[test]
    fn entries_take_their_first_members_place_and_keep_report_order() {
        let ignores = [
            directive(1, &["A"], Some("why")),
            directive(2, &["B"], Some("why")),
            directive(3, &["A"], Some("why")),
            directive(4, &["C"], Some("why")),
            directive(5, &["B"], Some("why")),
        ];
        assert_eq!(lines(&ignores, true), vec![vec![1, 3], vec![2, 5], vec![4]]);
        let grouped = entries(&ignores, true);
        assert!(grouped[0].is_group() && !grouped[2].is_group());
        assert_eq!(grouped[1].first().line, 2);
    }

    #[test]
    fn without_grouping_every_suppression_is_its_own_entry() {
        let ignores = [
            directive(1, &["A"], Some("why")),
            directive(2, &["A"], Some("why")),
        ];
        assert_eq!(lines(&ignores, false), vec![vec![1], vec![2]]);
        assert!(entries(&[], true).is_empty());
    }
}
