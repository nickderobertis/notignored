#!/usr/bin/env bash
# The three steps release.yml takes on the GitHub Release itself, under release
# immutability.
#
# With immutability on, a Release accepts no new, replaced or deleted asset once
# it is published, and its tag can no longer move. So the order is draft first:
# release-plz cuts the Release as a draft (`git_release_draft` in
# release-plz.toml), every `upload` leg attaches to that draft, and only then is
# it published — by this script, with the job's GITHUB_TOKEN, which fires no
# workflow, so publication cannot start the pipeline a second time.
#
#   await-draft       wait (bounded) for the Release for TAG to exist, and refuse
#                     one that is already published: nothing can be attached to
#                     it any more. Each `upload` leg runs this before attaching.
#   publish           publish the draft for TAG. A Release that is already
#                     published is left alone, so re-running the job is safe.
#   verify-immutable  read the published Release back and fail unless GitHub
#                     reports `"immutable": true`. `major-tag` waits on this, so
#                     `v0` never moves onto a Release whose tag could still move.
#
# Reads GH_TOKEN and GITHUB_REPOSITORY; GITHUB_API_URL points it at another API
# origin (GitHub Enterprise, or a test's loopback server).
#
# Quiet on success: one line. On failure: an `::error::` line and what to do.
#
# Usage:
#   github-release.sh await-draft --tag vX.Y.Z [--wait SECONDS] [--interval SECONDS]
#   github-release.sh publish --tag vX.Y.Z
#   github-release.sh verify-immutable --tag vX.Y.Z [--wait SECONDS] [--interval SECONDS]
#
# --wait bounds how long a command polls for GitHub to catch up (300s for the
# draft to appear, 60s for the read-back), --interval how often it asks.
set -euo pipefail

usage="run 'github-release.sh await-draft|publish|verify-immutable --tag vX.Y.Z [--wait SECONDS] [--interval SECONDS]'"

die() {
  echo "::error::$1" >&2
  echo "ACTION: $2" >&2
  exit 1
}

fail_usage() {
  echo "github-release: $1" >&2
  echo "ACTION: $usage" >&2
  exit 2
}

[ "$#" -gt 0 ] || fail_usage "no command given"
command="$1"
shift
interval=10
case "$command" in
  await-draft) wait=300 ;;
  verify-immutable) wait=60; interval=5 ;;
  publish) wait=0 ;;
  *) fail_usage "unknown command '$command'" ;;
esac

tag=""
while [ "$#" -gt 0 ]; do
  case "$1" in
    --tag)
      [ "$#" -ge 2 ] || fail_usage "--tag needs a value"
      tag="$2"
      shift 2
      ;;
    --wait)
      [ "$#" -ge 2 ] || fail_usage "--wait needs a value"
      case "$2" in
        "" | *[!0-9]*) fail_usage "--wait needs a whole number of seconds, not '$2'" ;;
      esac
      wait="$2"
      shift 2
      ;;
    --interval)
      [ "$#" -ge 2 ] || fail_usage "--interval needs a value"
      case "$2" in
        "" | *[!0-9]* | 0) fail_usage "--interval needs a whole number of seconds above 0, not '$2'" ;;
      esac
      interval="$2"
      shift 2
      ;;
    *) fail_usage "unknown option $1" ;;
  esac
done

# The tag is spliced into an API path and a jq filter, so bound it to the shape
# release-plz writes before either sees it.
if ! [[ "$tag" =~ ^v[0-9]+\.[0-9]+\.[0-9]+([-+][0-9A-Za-z.-]+)?$ ]]; then
  fail_usage "--tag must be a vX.Y.Z release tag, not '$tag'"
fi
case "${GITHUB_REPOSITORY:-}" in
  *[!A-Za-z0-9._/-]* | */*/* | *..* | "") repo_ok="" ;;
  ?*/?*) repo_ok="ok" ;;
  *) repo_ok="" ;;
esac
[ -n "$repo_ok" ] || die "GITHUB_REPOSITORY is not owner/repo: '${GITHUB_REPOSITORY:-}'" \
  "run this inside GitHub Actions, which sets it"
repo="$GITHUB_REPOSITORY"
api="${GITHUB_API_URL:-https://api.github.com}"
case "$api" in
  http://* | https://*) ;;
  *) die "GITHUB_API_URL is not an http(s) origin: '$api'" \
    "unset it to use https://api.github.com" ;;
esac
token_hint="give the job contents: write and pass its token as GH_TOKEN"

# `<id> <draft>` for the Release carrying TAG, or nothing when there is none yet.
# The list endpoint, not `releases/tags/…`: only the list returns drafts, and
# only to a token that can push.
find_release() {
  local found
  found="$(gh api --paginate "$api/repos/$repo/releases?per_page=100" \
    --jq ".[] | select(.tag_name == \"$tag\") | \"\(.id) \(.draft)\"")" \
    || die "cannot list the Releases of $repo" "$token_hint"
  printf '%s\n' "${found%%$'\n'*}"
}

SECONDS=0
case "$command" in
  await-draft)
    while :; do
      release="$(find_release)"
      if [ -n "$release" ]; then
        case "${release#* }" in
          true)
            echo "github-release: $tag is a draft; attaching to it"
            exit 0
            ;;
          *)
            die "the Release for $tag is already published, and under release immutability nothing can be attached to it" \
              "leave it as it is; the next release publishes its assets to a fresh draft (see AGENTS.md, 'Commits, releases, and merging')"
            ;;
        esac
      fi
      if [ "$SECONDS" -ge "$wait" ]; then
        die "no Release for $tag after ${SECONDS}s; release-plz pushes the tag and then cuts the draft, so the draft was never cut" \
          "read the release-plz run for the push that tagged $tag; the next release cuts a fresh draft"
      fi
      sleep "$interval"
    done
    ;;

  publish)
    release="$(find_release)"
    [ -n "$release" ] || die "no Release for $tag to publish" \
      "check that release-plz cut a draft for $tag (git_release_draft in release-plz.toml)"
    id="${release%% *}"
    case "$id" in
      '' | *[!0-9]*) die "the API named Release '$id', which is not an id" \
        "check GITHUB_API_URL — the host answering is not the GitHub API" ;;
    esac
    if [ "${release#* }" != "true" ]; then
      echo "github-release: $tag is already published; nothing to do"
      exit 0
    fi
    gh api --silent -X PATCH "$api/repos/$repo/releases/$id" -F draft=false \
      || die "cannot publish the draft Release for $tag" "$token_hint"
    echo "github-release: published $tag"
    ;;

  verify-immutable)
    # `releases/tags/…` answers only for a published Release, so a 404 here is a
    # draft (or no Release at all) — a different failure from a mutable one, and
    # told apart below so the advice matches it.
    answer="$(mktemp)"
    trap 'rm -f "$answer"' EXIT
    while :; do
      state="$(gh api "$api/repos/$repo/releases/tags/$tag" \
        --jq '"\(.draft) \(.immutable)"' 2>"$answer")" || state="unpublished"
      if [ "$state" = "false true" ]; then
        echo "github-release: $tag is published and immutable"
        exit 0
      fi
      if [ "$SECONDS" -ge "$wait" ]; then
        break
      fi
      sleep "$interval"
    done
    case "$state" in
      "false "*)
        die "the Release for $tag reads back \"immutable\": ${state#* }, so release immutability is off for $repo — its tag can still move, and v0 stays where it was" \
          "turn the setting on (Settings → General → Releases → Enable release immutability; 'gh api repos/$repo/immutable-releases' reads it) — it is not retroactive, so the next release is the first published under it"
        ;;
      *)
        cat "$answer" >&2
        die "no published Release for $tag reads back after ${SECONDS}s — it is still a draft, or the API cannot see it" \
          "run 'github-release.sh publish --tag $tag' first, and check GH_TOKEN can read $repo ($token_hint)"
        ;;
    esac
    ;;
esac
