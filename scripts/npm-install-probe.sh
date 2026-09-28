#!/usr/bin/env bash
# Install from npm, and fail unless the install brought this runner's platform
# package — the part of `notignored-cli` that actually carries the binary.
#
# `npm install` treats an optional dependency it cannot resolve as skippable and
# exits 0, so a bare install is the wrong probe for a just-published release: when
# the registry serves the launcher before the platform package it pins, the
# install "succeeds" with a launcher that has no binary to run.
# `tests/e2e/verify_npm.rs` records the releases that went red that way.
#
# So the probe is the install *plus* the one thing it can silently omit: this
# checks that the launcher resolves `notignored-cli-<platform>-<arch>` at exactly
# VERSION, the way `npm/notignored/bin/notignored.js` resolves it, and exits
# non-zero naming that package when it does not. Run under retry-install.sh, a
# platform package the registry has not served yet is then retried inside the
# same bounded budget as any other not-yet-served version.
#
# `--prefer-online` is always passed, for the reason retry-install.sh gives: npm
# would otherwise re-read the metadata it cached on the attempt that failed.
#
# The package name is the launcher's own rule — `notignored-cli-` plus
# `process.platform-process.arch`, the keys of its PACKAGES map —
# and tests/packaging_contract.rs holds every entry of that map to it.
#
# Quiet on success: npm's own output only. On failure: what to do, then a last
# line naming the missing package — the line retry-install.sh prints per attempt.
#
# Usage:
#   npm-install-probe.sh --version VERSION [--global] SPEC...
set -euo pipefail

version=""
global=""

usage="run 'npm-install-probe.sh --version VERSION [--global] SPEC...'"

fail_usage() {
  echo "npm-install-probe: $1" >&2
  echo "ACTION: $usage" >&2
  exit 2
}

while [ "$#" -gt 0 ]; do
  case "$1" in
    --version)
      [ "$#" -ge 2 ] || fail_usage "--version needs a value"
      version="$2"
      shift 2
      ;;
    --global) global="--global"; shift ;;
    -*) fail_usage "unknown option $1" ;;
    *) break ;;
  esac
done

[ -n "$version" ] || fail_usage "--version is required: the platform package must be at the release's exact version"
# Compared against an installed manifest and named in every failure, so held to
# the X.Y.Z[-pre] shape a release version has.
[[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+([-+][0-9A-Za-z.-]+)?$ ]] \
  || fail_usage "--version must be a release version such as 1.2.3, not '$version'"
[ "$#" -gt 0 ] || fail_usage "no package to install"

# npm's own error comes first; the last line is ours, because it is the one
# retry-install.sh shows per attempt.
if ! npm install ${global:+"$global"} --prefer-online "$@"; then
  echo "ACTION: read npm's error above — a version the registry does not serve yet is retried; anything else needs fixing" >&2
  echo "npm-install-probe: npm install $* failed" >&2
  exit 1
fi

# Where the install landed: the global tree, or this directory's node_modules.
if ! root="$(npm root ${global:+"$global"})"; then
  echo "ACTION: check the npm on PATH works ('npm root${global:+ $global}')" >&2
  echo "npm-install-probe: cannot ask npm where it installed to" >&2
  exit 1
fi

# Resolved from the installed launcher's own directory, exactly as the launcher
# resolves it at run time — so a package npm hoisted, nested, or skipped is judged
# by the rule that decides whether `notignored` will run.
# The single-quoted program is JavaScript; its template expressions are not shell.
# shellcheck disable=SC2016
node -e '
  const fs = require("node:fs");
  const path = require("node:path");
  const [root, expected] = process.argv.slice(1);
  const pkg = `notignored-cli-${process.platform}-${process.arch}`;
  const fail = (message, action) => {
    process.stderr.write(`ACTION: ${action}\n`);
    process.stderr.write(`npm-install-probe: ${message}\n`);
    process.exit(1);
  };
  const versionOf = (manifest) => {
    try {
      return JSON.parse(fs.readFileSync(manifest, "utf8")).version;
    } catch (error) {
      fail(`cannot read ${manifest}: ${error.message}`, "the install is damaged; retrying reinstalls it");
    }
  };
  let launcher;
  try {
    launcher = require.resolve("notignored-cli/package.json", { paths: [root] });
  } catch {
    fail(
      `notignored-cli is not installed under ${root}`,
      "pass notignored-cli@<version> among the packages to install"
    );
  }
  let manifest;
  try {
    manifest = require.resolve(`${pkg}/package.json`, { paths: [path.dirname(launcher)] });
  } catch {
    fail(
      `npm installed notignored-cli but not its platform package ${pkg}@${expected} — the registry may not serve ${pkg}@${expected} yet`,
      `check https://www.npmjs.com/package/${pkg} lists ${expected}; publish-npm publishes it, and a retry installs it once the registry serves it`
    );
  }
  const installed = versionOf(manifest);
  if (installed !== expected) {
    fail(
      `npm installed ${pkg}@${installed}, not ${pkg}@${expected}`,
      `install notignored-cli@${expected}, whose launcher pins ${pkg}@${expected}, rather than another version of it`
    );
  }
' "$root" "$version"
