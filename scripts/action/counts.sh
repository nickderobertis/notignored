#!/usr/bin/env bash
# Count what a `notignored --diff` report says the change did, and set the two
# outputs a calling workflow gates on.
#
# `count` keeps its documented meaning — suppressions the change added — so a
# build gating on it does not start failing the day somebody rewords a
# justification. The rewritten justifications are counted beside it, never into
# it, and the two counts partition the report between them: a record whose
# `change` is `justification-edited` is one, and every other record is an
# addition. That is what keeps a record this version has no word for — one an
# older notignored wrote without the field, one a newer one wrote with a word
# this version never heard of — reported rather than dropped silently out of
# both numbers.
#
# Reads REPORT (the JSON report the scan wrote) and writes `count` and
# `justification-edited-count` to $GITHUB_OUTPUT, then says what it found.
set -euo pipefail

die() {
    printf 'notignored: %s\n' "$1" >&2
    printf 'ACTION: %s\n' "$2" >&2
    exit 1
}

REPORT="${REPORT:-}"
[ -n "$REPORT" ] || die "REPORT is not set" \
    "point it at the JSON report the scan step wrote"
[ -f "$REPORT" ] || die "report $REPORT does not exist" \
    "check that the scan step ran and wrote its report"

# The report is JSON this run produced a moment ago, but it is still read as
# data: a truncated write or a build whose envelope moved would otherwise be
# counted as zero findings, which reads exactly like a clean pull request.
jq -e '.ignores | type == "array"' "$REPORT" >/dev/null 2>&1 || die \
    "report $REPORT has no ignores array" \
    "check that the scan step completed; a truncated report must not count as a clean one"

edited="$(jq '[.ignores[] | select(.change == "justification-edited")] | length' "$REPORT")"
count="$(jq '[.ignores[] | select(.change != "justification-edited")] | length' "$REPORT")"

if [ -n "${GITHUB_OUTPUT:-}" ]; then
    {
        echo "count=$count"
        echo "justification-edited-count=$edited"
    } >> "$GITHUB_OUTPUT"
fi
printf 'notignored: %s suppression(s) added, %s justification(s) edited\n' "$count" "$edited"
