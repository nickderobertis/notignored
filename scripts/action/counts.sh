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
# data, and the whole shape the counting rests on is checked before any of it is
# counted: an array of records, each of them an object, each `change` a string.
# A truncated write or a build whose envelope moved would otherwise count as
# zero findings, which reads exactly like a clean pull request.
# jq's own diagnostic is kept and reported: a filter that answered "no" says
# nothing on stderr, while a jq that is missing or broken says exactly what went
# wrong, and the two failures need different fixing.
if ! refused="$(jq -e '(.ignores | type == "array")
                       and all(.ignores[];
                               (type == "object")
                               and ((has("change") | not)
                                    or (.change | type == "string")))' \
    "$REPORT" 2>&1 >/dev/null)"; then
    die "report $REPORT is not a report of suppression records${refused:+: $refused}" \
        "check that the scan step completed and wrote the whole envelope, and that \
jq is on PATH; a truncated report must not count as a clean one"
fi

COUNT_HINT="re-run the scan step; the report it wrote cannot be counted"
edited="$(jq '[.ignores[] | select(.change == "justification-edited")] | length' "$REPORT")" \
    || die "cannot count the rewritten justifications in $REPORT" "$COUNT_HINT"
count="$(jq '[.ignores[] | select(.change != "justification-edited")] | length' "$REPORT")" \
    || die "cannot count the added suppressions in $REPORT" "$COUNT_HINT"

if [ -n "${GITHUB_OUTPUT:-}" ]; then
    {
        echo "count=$count"
        echo "justification-edited-count=$edited"
    } >> "$GITHUB_OUTPUT" || die "cannot write the counts to $GITHUB_OUTPUT" \
        "run this step inside GitHub Actions, which sets a writable output file"
fi
printf 'notignored: %s suppression(s) added, %s justification(s) edited\n' "$count" "$edited"
