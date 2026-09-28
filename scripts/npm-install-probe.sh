#!/usr/bin/env bash
# Install from npm, and fail unless the install brought this runner's platform
# package — the part of `notignored-cli` that actually carries the binary.
#
# `npm install` treats an optional dependency it cannot resolve as skippable and
# exits 0. That made a bare install the wrong probe for a just-published release:
# on v0.1.13, v0.1.14, v0.1.15 and v0.1.16, `publish-npm` published
# `notignored-cli-darwin-arm64` *first* and npm acknowledged it, yet the registry
# only recorded it a minute or two later — after the launcher (v0.1.16: publish
# acknowledged 18:41:14Z, registry `time` 18:42:19Z, launcher 18:41:28Z). The
# macOS arm64 verify leg installed at 18:41:43Z, npm skipped the platform package
# it could not yet resolve, `scripts/retry-install.sh` accepted that exit 0 on the
# first attempt, and the smoke test then found a launcher with no binary. Every
# other leg's platform package had landed in time.
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
# Quiet on success: npm's own output only. On failure the last line names the
# missing package, which is what retry-install.sh prints per attempt.
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
[ "$#" -gt 0 ] || fail_usage "no package to install"

npm install ${global:+"$global"} --prefer-online "$@"

# Where the install landed: the global tree, or this directory's node_modules.
root="$(npm root ${global:+"$global"})"

# Resolved from the installed launcher's own directory, exactly as the launcher
# resolves it at run time — so a package npm hoisted, nested, or skipped is judged
# by the rule that decides whether `notignored` will run.
# The single-quoted program is JavaScript; its template expressions are not shell.
# shellcheck disable=SC2016
node -e '
  const path = require("node:path");
  const [root, expected] = process.argv.slice(1);
  const pkg = `notignored-cli-${process.platform}-${process.arch}`;
  const fail = (message) => {
    process.stderr.write(`npm-install-probe: ${message}\n`);
    process.exit(1);
  };
  let launcher;
  try {
    launcher = require.resolve("notignored-cli/package.json", { paths: [root] });
  } catch {
    fail(`notignored-cli is not installed under ${root}`);
  }
  let manifest;
  try {
    manifest = require.resolve(`${pkg}/package.json`, { paths: [path.dirname(launcher)] });
  } catch {
    fail(`npm installed notignored-cli but not its platform package ${pkg}@${expected} — the registry may not serve ${pkg}@${expected} yet`);
  }
  const installed = require(manifest).version;
  if (installed !== expected) {
    fail(`npm installed ${pkg}@${installed}, not ${pkg}@${expected}`);
  }
' "$root" "$version"
