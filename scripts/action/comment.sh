#!/usr/bin/env bash
# Upsert the one sticky notignored comment on a pull request.
#
# The comment is found by the hidden marker `notignored --format markdown` opens
# every body with, so a pull request accumulates a single comment that is edited
# in place rather than one comment per push. With nothing to report and no
# previous comment, nothing is posted at all: a pull request that adds no
# suppressions and rewrites no justification stays clean.
#
# Reads BODY_FILE (the rendered body), COUNT (how many suppressions the change
# added), and GH_TOKEN. JUSTIFICATION_EDITED_COUNT — how many existing
# suppressions had their justification rewritten — is optional and defaults to 0,
# so a caller that predates it behaves exactly as it did. PR_NUMBER is taken from
# the event payload when it is not set.
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

# Every call to the API can fail for the same two reasons, and gh reports the
# status without saying which knob fixes it.
TOKEN_HINT="grant the job pull-requests: write and pass a token with it (github-token)"

# Every failure leaves through `die`, so a missing input reads the same way a bad
# one does: the cause, then the concrete next action. Bash's own `${VAR:?}` says
# only that something is unset, which is the half a reader already knows.
[ -n "${GITHUB_REPOSITORY:-}" ] || die "GITHUB_REPOSITORY is not set" \
    "run this step inside GitHub Actions, which sets it"
# Both of these are interpolated into an API path, so they are bounded to their
# documented shapes here rather than trusted: an owner/repo slug of two plain
# segments, and an http(s) origin. `src/cli/mod.rs` bounds the same slug for the
# permalinks, and `scripts/install.sh` bounds the same kind of URL override.
case "$GITHUB_REPOSITORY" in
    # Anything outside the character set GitHub allows in an owner or a repo
    # name — a `?`, a `#`, a `%`, whitespace — would redirect the request or
    # truncate the path. A third segment, an empty one, or a `..` would too.
    *[!A-Za-z0-9._/-]* | */*/* | *..*) slug='' ;;
    ?*/?*) slug='ok' ;;
    *) slug='' ;;
esac
[ -n "$slug" ] || die "GITHUB_REPOSITORY is not owner/repo: '$GITHUB_REPOSITORY'" \
    "run this step inside GitHub Actions, which sets it"
BODY_FILE="${BODY_FILE:-}"
[ -n "$BODY_FILE" ] || die "BODY_FILE is not set" \
    "point it at the body the scan step rendered"
COUNT="${COUNT:-}"
[ -n "$COUNT" ] || die "COUNT is not set" "pass the scan step's count output"
# Optional, and zero when it is missing: a workflow calling this script without
# it is a caller from before the count existed, not a broken one.
EDITED="${JUSTIFICATION_EDITED_COUNT:-0}"
API="${GITHUB_API_URL:-https://api.github.com}"
case "$API" in
    http://*|https://*) ;;
    *) die "GITHUB_API_URL is not an http(s) origin: '$API'" \
        "unset it to use https://api.github.com, or point it at your GitHub Enterprise API" ;;
esac

[ -f "$BODY_FILE" ] || die "comment body $BODY_FILE does not exist" \
    "check that the scan step ran and wrote its body"
case "$COUNT" in
    '' | *[!0-9]*) die "COUNT is not a count: '$COUNT'" "pass the scan step's count output" ;;
esac
case "$EDITED" in
    '' | *[!0-9]*) die "JUSTIFICATION_EDITED_COUNT is not a count: '$EDITED'" \
        "pass the scan step's justification-edited-count output, or leave it unset" ;;
esac

# The pull request to comment on. The payload is read as data — never
# interpolated into this script — so a fork's branch names cannot reach the shell.
NUMBER="${PR_NUMBER:-}"
if [ -z "$NUMBER" ] && [ -f "${GITHUB_EVENT_PATH:-}" ]; then
    NUMBER="$(jq -r '.pull_request.number // .issue.number // empty' "$GITHUB_EVENT_PATH")" \
        || die "cannot read the pull request number from $GITHUB_EVENT_PATH" \
            "run this action on a pull_request event, or set PR_NUMBER yourself"
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
    --jq "map(select((.body // \"\") | contains(\"$MARKER\"))) | .[0].id // empty")" \
    || die "cannot list the comments on #$NUMBER" "$TOKEN_HINT"
existing="${found%%$'\n'*}"

tally="$COUNT suppression(s) added, $EDITED justification(s) edited"

if [ -n "$existing" ]; then
    gh api --silent -X PATCH "$API/repos/$GITHUB_REPOSITORY/issues/comments/$existing" \
        -F "body=@$BODY_FILE" \
        || die "cannot update comment $existing" "$TOKEN_HINT"
    printf 'notignored: updated comment %s (%s)\n' "$existing" "$tally"
# Anything to report earns a comment, added or rejustified. Posting only on
# additions would say nothing at all about a pull request that rewrote every
# justification it touched — the very confusion this comment exists to remove.
elif [ "$COUNT" -gt 0 ] || [ "$EDITED" -gt 0 ]; then
    gh api --silent -X POST "$API/repos/$GITHUB_REPOSITORY/issues/$NUMBER/comments" \
        -F "body=@$BODY_FILE" \
        || die "cannot comment on #$NUMBER" "$TOKEN_HINT"
    printf 'notignored: commented on #%s (%s)\n' "$NUMBER" "$tally"
else
    printf 'notignored: no suppressions added, no justifications edited, and no comment to update — posted nothing\n'
fi
