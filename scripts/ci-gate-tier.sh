#!/usr/bin/env bash
# Which tier ci.yml gates this event on, as `$GITHUB_OUTPUT` lines:
#
#   tier=affected|full   `just check-affected`, or the full sweep `just check`
#   base=<sha>           the explicit base commit for the affected tier, or empty
#                        to take the pull request's merge base
#
# Releases are batched: release-plz's release pull request accumulates every
# merge since the last tag, so the commit that ships is one no merge job swept.
# The broader tier therefore runs once, at release-prep — on that pull request —
# and merge-to-main stays on the affected tier, keyed on the pushed range's base
# (`github.event.before`) rather than on a base branch a push build does not
# have. AGENTS.md "Commits, releases, and merging" records the model.
#
# Reads GATE_EVENT (`github.event_name`), GATE_HEAD_REF (`github.head_ref`) and
# GATE_BEFORE (`github.event.before`), all passed through `env:` and never
# spliced into a script. A fork can name its branch like release-plz's; that buys
# it the full sweep, which proves more, never less.
set -euo pipefail

# release-plz names its release branches `release-plz-<timestamp>`; release-plz.yml
# finds its own pull request by the same prefix.
readonly RELEASE_BRANCH_PREFIX="release-plz-"

die() {
  printf 'ci-gate-tier: %s\n' "$1" >&2
  printf 'ACTION: %s\n' "$2" >&2
  exit 1
}

event="${GATE_EVENT:-}"
case "$event" in
pull_request)
  case "${GATE_HEAD_REF:-}" in
  "$RELEASE_BRANCH_PREFIX"*) printf 'tier=full\nbase=\n' ;;
  *) printf 'tier=affected\nbase=\n' ;;
  esac
  ;;
push)
  before="${GATE_BEFORE:-}"
  # Written into $GITHUB_OUTPUT, so a value that could carry a newline would
  # forge an output. Whether it names a commit is nx-affected.sh's to decide,
  # and it selects every project when it does not.
  # Matched as one string, not line by line as grep would.
  case "$before" in
  *[!0-9A-Fa-f]*)
    die "GATE_BEFORE '$before' is not a commit id" \
      "pass \`github.event.before\` unaltered as GATE_BEFORE"
    ;;
  esac
  [ -n "$before" ] || echo "ci-gate-tier: no pushed-range base; the affected tier will select every project" >&2
  printf 'tier=affected\nbase=%s\n' "$before"
  ;;
*)
  die "GATE_EVENT is '${event}', which ci.yml does not gate on" \
    "pass \`github.event_name\` as GATE_EVENT; a new trigger needs its tier decided here"
  ;;
esac
