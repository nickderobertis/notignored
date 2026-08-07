#!/usr/bin/env bash
# Move the floating major tag (`v0`, `v1`, …) onto a release that fully shipped.
#
# `uses: owner/action@v0` is what the Actions ecosystem expects a consumer to
# write: a ref that follows patches and minors but never a breaking change, and
# never unreleased work. This repository only ever had exact `vX.Y.Z` tags from
# release-plz, so the README told consumers `@main` — a mutable branch carrying
# every merged commit, released or not.
#
# The major is DERIVED from the release tag, never configured: `v0.1.10` moves
# `v0`, and the first `1.0.0` starts moving `v1` with no edit here. Two things it
# refuses to do, because a floating ref is resolved by consumers who never see
# this run:
#
#   * a pre-release (`v1.0.0-rc.1`) leaves the major tag alone — `@v1` must mean
#     the newest stable release, not whatever was cut last;
#   * an OLDER release than the newest one with that major leaves it alone too.
#     Re-cutting `v0.1.4` after `v0.1.10` shipped (a re-run, or a patch off an
#     old branch) would otherwise walk every `@v0` consumer backwards.
#
# WHEN it runs is the other half of the contract and belongs to the caller:
# `.github/workflows/release.yml`'s `major-tag` job waits on every publish and
# verify job, so `v0` can never resolve to a release whose artifacts failed to
# publish. Run by hand only to repair a tag a failed release left behind.
#
# Quiet on success: one line naming what moved, or what it deliberately did not.
#
# Usage:
#   update-major-tag.sh --tag vX.Y.Z [--remote NAME]
set -euo pipefail

remote="origin"
tag=""

usage="run 'update-major-tag.sh --tag vX.Y.Z [--remote NAME]'"

fail() {
  echo "update-major-tag: $1" >&2
  echo "ACTION: $2" >&2
  exit 1
}

need_value() {
  if [ "$#" -lt 2 ]; then
    fail "$1 needs a value" "$usage"
  fi
}

while [ "$#" -gt 0 ]; do
  case "$1" in
    --tag)
      need_value "$@"
      tag="$2"
      shift 2
      ;;
    --remote)
      need_value "$@"
      remote="$2"
      shift 2
      ;;
    -h | --help)
      echo "$usage"
      exit 0
      ;;
    *)
      fail "unknown argument '$1'" "$usage"
      ;;
  esac
done

if [ -z "$tag" ]; then
  fail "no release tag given" "$usage"
fi

if ! git rev-parse --git-dir >/dev/null 2>&1; then
  fail "not inside a git repository" "run this from a checkout of the repository being released"
fi

# The tag reaches git as a revision and the remote as a refspec, so validate its
# whole shape first. A glob cannot say "digits and nothing else" — the same
# reasoning, and the same pattern, as release.yml's own tag check.
if ! printf '%s' "$tag" | grep -Eq '^v[0-9]+\.[0-9]+\.[0-9]+([-+][0-9A-Za-z.-]+)?$'; then
  fail "release tag '$tag' is not a vX.Y.Z version tag" \
    "release-plz tags vX.Y.Z; a Release cut by hand with any other tag is not one this pipeline built"
fi

# `v1.0.0-rc.1` and `v1.0.0+build.3` are both releases the floating major must
# not follow. Reachable only for tags the pattern above already accepted, so the
# two cases below are the whole suffix grammar.
case "$tag" in
  *-* | *+*)
    echo "update-major-tag: $tag is a pre-release; the floating major tag still points at the newest stable release."
    exit 0
    ;;
esac

major="${tag%%.*}"

# Every tag, not just the checked-out one: the newest-release comparison below is
# only as good as the set of tags this clone can see, and a checkout that fetched
# none would conclude every release is the newest.
if ! fetch_error="$(git fetch --force --tags "$remote" 2>&1)"; then
  echo "$fetch_error" >&2
  fail "cannot fetch tags from '$remote'" \
    "check the remote exists and the token can read it"
fi

if ! sha="$(git rev-list --max-count=1 "$tag" 2>&1)"; then
  echo "$sha" >&2
  fail "no commit for tag '$tag'" \
    "the release tag must exist on '$remote' before its major tag can point at it"
fi

# The newest stable release sharing this major, compared numerically — `sort`'s
# default lexical order puts v0.1.9 above v0.1.10. Pre-release tags are filtered
# out rather than ordered: they are not candidates for a floating major ref.
newest="$(
  git tag --list "$major.*" \
    | { grep -E '^v[0-9]+\.[0-9]+\.[0-9]+$' || true; } \
    | sed 's/^v//' \
    | sort -t. -k1,1n -k2,2n -k3,3n \
    | tail -n 1
)"

if [ "v$newest" != "$tag" ]; then
  echo "update-major-tag: v$newest is a newer $major release than $tag; leaving $major where it is."
  exit 0
fi

git tag --force "$major" "$sha" >/dev/null

if ! push_error="$(git push --force "$remote" "refs/tags/$major" 2>&1)"; then
  echo "$push_error" >&2
  fail "cannot update '$major' on '$remote'" \
    "the job needs 'permissions: contents: write', and the tag must not be protected by a ruleset"
fi

echo "update-major-tag: $major -> $tag ($(printf '%.7s' "$sha"))"
