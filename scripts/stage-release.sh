#!/usr/bin/env bash
# Install the staged wheel, npm platform+launcher, and checksummed archive,
# then run the same smoke assertion as registry delivery. Never publishes.
set -Eeuo pipefail
die() {
  echo "stage-release: $1" >&2
  echo 'ACTION: rebuild the staged target and inspect its metadata, payload and checksum before rerunning' >&2
  exit 1
}
on_error() {
  local code="$?"
  echo "stage-release: operation failed at line $LINENO (exit $code)" >&2
  echo 'ACTION: inspect the reported installer error and rebuild this target before rerunning' >&2
  exit "$code"
}
trap on_error ERR
run() {
  if ! "$@" >"$work/operation.log" 2>&1; then
    cat "$work/operation.log" >&2
    die "$1 failed"
  fi
}
[ "$#" -ge 2 ] && [ -n "$1" ] && [ -n "$2" ] || {
  echo 'stage-release: expected a staged target directory and Python interpreter' >&2
  echo 'ACTION: run bash scripts/stage-release.sh <staged-target-directory> <python-interpreter> [sdk-tarball]' >&2
  exit 1
}
stage="$1"
python="$2"
sdk="${3:-}"
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
stage="$(cd "$stage" && pwd)"
manifest_fields="$(node --input-type=module -e '
  import { readFileSync } from "node:fs";
  import { pathToFileURL } from "node:url";
  const { isReleaseVersion } = await import(pathToFileURL(process.argv[2]).href);
  const manifest = JSON.parse(readFileSync(process.argv[1]));
  if (!manifest || typeof manifest !== "object" || typeof manifest.target !== "string"
      || !isReleaseVersion(manifest.version)
      || manifest.tag !== "v" + manifest.version
      || !manifest.hashes || Object.keys(manifest.hashes).sort().join() !== "archive,npm,wheel"
      || Object.values(manifest.hashes).some(hash => !/^[0-9a-f]{64}$/.test(hash))
      || new Set(Object.values(manifest.hashes)).size !== 1
      || [manifest.wheel, manifest.npm].some(name => typeof name !== "string"
        || !/^[^/\\\x00-\x1f\x7f]+$/.test(name) || name === "." || name === "..")) throw new Error("invalid staged manifest");
  console.log([manifest.version, manifest.wheel, manifest.npm].join("\n"));
' "$stage/manifest.json" "$root/scripts/release-version.mjs")"
# Install exactly the artifacts the manifest names and stage-release.py verifies,
# never whatever else a glob would find beside them.
{ read -r version; read -r wheel; read -r npm_artifact; } <<< "$manifest_fields"
work="$(mktemp -d "${TMPDIR:-/tmp}/notignored-stage.XXXXXXXX")"
trap 'rm -rf "$work"' EXIT
run python3 "$root/scripts/stage-release.py" --verify --out "$stage"
run uv venv --quiet --python "$python" "$work/venv"
run uv pip install --quiet --offline --python "$work/venv" "$stage/wheels/$wheel"
PATH="$work/venv/bin:$PATH" run bash "$root/scripts/smoke-published.sh" --expect-version "$version" --label "staged wheel"
launcher="$(node "$root/scripts/npm-build.mjs" launcher --out "$work/npm")"
(cd "$launcher" && npm pack --silent --pack-destination "$work" >/dev/null)
run npm install --offline --no-audit --no-fund --prefix "$work/npm-install" "$stage/npm-artifacts/$npm_artifact" "$work"/*.tgz
PATH="$work/npm-install/node_modules/.bin:$PATH" run bash "$root/scripts/smoke-published.sh" --expect-version "$version" --label "staged npm"
if [ -n "$sdk" ]; then
  run npm install --offline --no-audit --no-fund --prefix "$work/npm-install" "$sdk"
  printf 'x = 1  # noqa: E501  # smoke test\n' > "$work/npm-install/app.py"
  (cd "$work/npm-install" && PATH="$work/npm-install/node_modules/.bin:$PATH" node --input-type=module -e '
    import { scan } from "notignored-sdk";
    const report = await scan(["app.py"]);
    const [d] = report.ignores;
    if (report.ignores.length !== 1 || d.tool !== "ruff" || d.rules[0] !== "E501" || d.reason !== "smoke test") throw new Error(JSON.stringify(report));
  ')
fi
# The installer's network boundary is owned by a local HTTP server serving the
# staged archive and its exact .sha256 sibling. Only this invocation's PID is
# signalled; no public release or registry is needed.
python3 "$root/scripts/staged-transport.py" "$stage/archives" "$work/port" >"$work/server.log" 2>&1 &
server_pid=$!
trap 'kill "$server_pid" 2>/dev/null || true; wait "$server_pid" 2>/dev/null || true; rm -rf "$work"' EXIT
startup_deadline=$((SECONDS + 30))
while [ "$SECONDS" -lt "$startup_deadline" ]; do
  [ ! -s "$work/port" ] || break
  # llmlint: ignore[changed_behavior_has_e2e] OS/import startup termination is outside the deterministic transport API. Real invalid transport arguments prove its refusal diagnostics, and real artifact failures prove cleanup; rewriting the transport invocation to force this guard would mock the layer under test.
  kill -0 "$server_pid" 2>/dev/null || { cat "$work/server.log" >&2; die "staged transport stopped before startup"; }
  sleep 0.05
done
# llmlint: ignore[changed_behavior_has_e2e] OS startup starvation is outside the deterministic transport API. Real delayed-transport recovery and artifact failures prove installation and cleanup; replacing the transport to force this deadline would mock the layer under test.
[ -s "$work/port" ] || { cat "$work/server.log" >&2; die "local transport did not start within 30 seconds"; }
port="$(cat "$work/port")" || die "cannot read staged transport port"
# llmlint: ignore-block[changed_behavior_has_e2e] the real transport publishes its port atomically and numerically, so only a replaced transport could write a bad one, which would mock the layer under test; this guard keeps a corrupt port file from reaching the installer URL.
case "$port" in
  '' | *[!0-9]*) die "staged transport wrote a non-numeric port: $port" ;;
esac
[ "$port" -ge 1 ] && [ "$port" -le 65535 ] || die "staged transport wrote an out-of-range port: $port"
# llmlint: ignore-end[changed_behavior_has_e2e]
NOTIGNORED_RELEASE_BASE_URL="http://127.0.0.1:$port" run sh "$root/scripts/install.sh" --version "v$version" --to "$work/archive"
PATH="$work/archive:$PATH" run bash "$root/scripts/smoke-published.sh" --expect-version "$version" --label "staged archive"
echo 'stage-release: all staged install surfaces passed'
