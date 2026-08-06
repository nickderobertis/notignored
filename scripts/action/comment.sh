#!/usr/bin/env bash
# Upsert the one sticky notignored comment on a pull request.
#
# The comment is found by the hidden marker `notignored --format markdown` opens
# every body with, so a pull request accumulates a single comment that is edited
# in place rather than one comment per push. With nothing to report and no
# previous comment, nothing is posted at all: a pull request that adds no
# suppressions stays clean.
#
# Reads BODY_FILE (the rendered body), COUNT (how many suppressions it names),
# and GH_TOKEN. PR_NUMBER is taken from the event payload when it is not set.
set -euo pipefail

# The marker `notignored`'s markdown renderer writes; tests/action_contract.rs
# fails the build if the two ever disagree. Keep the line below verbatim.
# STICKY_MARKER: <!-- notignored-report -->
MARKER='<!-- notignored-report -->'

die() {
    printf 'notignored: %s\n' "$1" >&2
    printf 'ACTION: %s\n' "$2" >&2
    exit 1
}

: "${GITHUB_REPOSITORY:?GITHUB_REPOSITORY is not set (run this inside GitHub Actions)}"
BODY_FILE="${BODY_FILE:?BODY_FILE is not set}"
COUNT="${COUNT:?COUNT is not set}"
API="${GITHUB_API_URL:-https://api.github.com}"

[ -f "$BODY_FILE" ] || die "comment body $BODY_FILE does not exist" \
    "check that the scan step ran and wrote its body"
case "$COUNT" in
    '' | *[!0-9]*) die "COUNT is not a count: '$COUNT'" "pass the scan step's count output" ;;
esac

# The pull request to comment on. The payload is read as data — never
# interpolated into this script — so a fork's branch names cannot reach the shell.
NUMBER="${PR_NUMBER:-}"
if [ -z "$NUMBER" ] && [ -f "${GITHUB_EVENT_PATH:-}" ]; then
    NUMBER="$(jq -r '.pull_request.number // .issue.number // empty' "$GITHUB_EVENT_PATH")"
fi
if [ -z "$NUMBER" ]; then
    printf 'notignored: not a pull request event — no comment to upsert\n'
    exit 0
fi
case "$NUMBER" in
    '' | *[!0-9]*) die "pull request number is not a number: '$NUMBER'" \
        "run this action on a pull_request event" ;;
esac

# `--paginate` repeats the filter per page, so several ids can come back; the
# first is the sticky comment and any other is a duplicate an older run left.
# Captured whole rather than piped through `head`, which would close gh's pipe
# and fail the script under `pipefail`.
found="$(gh api --paginate "$API/repos/$GITHUB_REPOSITORY/issues/$NUMBER/comments" \
    --jq "map(select((.body // \"\") | contains(\"$MARKER\"))) | .[0].id // empty")"
existing="${found%%$'\n'*}"

if [ -n "$existing" ]; then
    gh api --silent -X PATCH "$API/repos/$GITHUB_REPOSITORY/issues/comments/$existing" \
        -F "body=@$BODY_FILE"
    printf 'notignored: updated comment %s (%s suppression(s))\n' "$existing" "$COUNT"
elif [ "$COUNT" -gt 0 ]; then
    gh api --silent -X POST "$API/repos/$GITHUB_REPOSITORY/issues/$NUMBER/comments" \
        -F "body=@$BODY_FILE"
    printf 'notignored: commented on #%s (%s suppression(s))\n' "$NUMBER" "$COUNT"
else
    printf 'notignored: no suppressions added and no comment to update — posted nothing\n'
fi
