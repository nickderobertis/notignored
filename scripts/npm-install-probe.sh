#!/usr/bin/env bash
# Install from npm, and fail unless the install brought this runner's platform
# package — the part of `notignored-cli` that actually carries the binary.
#
# npm skips an optional dependency it cannot resolve yet and exits 0, so after a
# publish a bare install can "succeed" with a launcher whose binary never
# arrived (`tests/e2e/verify_npm.rs` records when). This also fails until the
# launcher resolves `notignored-cli-<process.platform>-<process.arch>` — the
# shim's own rule, which tests/packaging_contract.rs holds it to — at exactly
# VERSION, so retry-install.sh keeps waiting for it. Always `--prefer-online`, so
# a retry does not re-read the metadata npm cached on the failed attempt.
#
# Quiet on success. On failure: what to do, then a last line naming what is
# missing — the line retry-install.sh prints per attempt.
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
# Every remaining argument is a package spec: an option here would reach npm and
# change what or where it installs.
for spec in "$@"; do
  case "$spec" in
    -*) fail_usage "'$spec' is not a package spec; options go before the specs" ;;
  esac
done
command -v node >/dev/null 2>&1 || fail_usage "node is not on PATH; the install check runs in it (actions/setup-node)"

# npm's own error comes first; the last line is ours, because it is the one
# retry-install.sh shows per attempt.
if ! npm install ${global:+"$global"} --prefer-online "$@"; then
  echo "ACTION: an E404 or ETARGET above means the registry does not serve that version yet, and retry-install.sh retries it; for an auth, network or disk error, fix that and re-run the job" >&2
  echo "npm-install-probe: npm install $* failed" >&2
  exit 1
fi

# Where the install landed: the global tree, or this directory's node_modules.
# llmlint: ignore[changed_behavior_has_e2e] `npm root` failing straight after the same npm installed successfully cannot be staged without replacing npm with a stub, and a journey over a stub would prove the stub; the branch only turns that into a named failure.
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
  const path = require("node:path");
  const [root, expected] = process.argv.slice(1);
  const pkg = `notignored-cli-${process.platform}-${process.arch}`;
  const fail = (message, action) => {
    process.stderr.write(`ACTION: ${action}\n`);
    process.stderr.write(`npm-install-probe: ${message}\n`);
    process.exit(3);
  };
  let launcher;
  try {
    launcher = require.resolve("notignored-cli/package.json", { paths: [root] });
  } catch (error) {
    fail(
      `notignored-cli is not installed under ${root} (${error.message.split("\n")[0]})`,
      "pass notignored-cli@<version> among the packages to install"
    );
  }
  let manifest;
  try {
    manifest = require.resolve(`${pkg}/package.json`, { paths: [path.dirname(launcher)] });
  } catch (error) {
    fail(
      `npm installed notignored-cli but not its platform package ${pkg}@${expected} — the registry may not serve ${pkg}@${expected} yet (${error.message.split("\n")[0]})`,
      `check https://www.npmjs.com/package/${pkg} lists ${expected}; publish-npm publishes it, and a retry installs it once the registry serves it`
    );
  }
  let installed;
  // llmlint: ignore[changed_behavior_has_e2e] npm validates a package manifest when it unpacks it, so an installed platform package whose package.json will not parse cannot be staged through a real install; this turns that damage into a named failure rather than node setup advice.
  try {
    installed = require(manifest).version;
  } catch (error) {
    fail(`cannot read ${manifest}: ${error.message.split("\n")[0]}`, "the install is damaged; the retry reinstalls it");
  }
  if (installed !== expected) {
    fail(
      `npm installed ${pkg}@${installed}, not ${pkg}@${expected}`,
      `install notignored-cli@${expected}, whose launcher pins ${pkg}@${expected}, rather than another version of it`
    );
  }
' "$root" "$version" || {
  status=$?
  # The program's own failures have printed their ACTION and exit 3; any other
  # status is node failing to run the check at all.
  if [ "$status" -ne 3 ]; then
    echo "ACTION: check that 'node --version' runs on this runner (actions/setup-node)" >&2
    echo "npm-install-probe: node exited $status while checking the install" >&2
  fi
  exit 1
}
