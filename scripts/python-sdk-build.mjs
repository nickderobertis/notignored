#!/usr/bin/env node
// Assemble the `notignored-sdk` PyPI package at the crate's version — the direct
// analogue of what scripts/npm-build.mjs does for the npm launcher.
//
// The SDK is pure Python, so there is nothing to compile; the only thing a
// release has to do is stamp the two numbers the committed manifest deliberately
// leaves as placeholders:
//
//   version = "0.0.0.dev0"        ->  version = "<Cargo.toml version>"
//   dependencies = [".. -cli"]    ->  "notignored-cli==<Cargo.toml version>"
//
// Cargo.toml is this repository's only version source (see AGENTS.md, "The
// registry packages"), and the exact `notignored-cli` pin is what makes
// `pip install notignored-sdk` bring a binary that speaks the report contract
// this SDK reads. A real version in the committed manifest would be a second
// source that silently went stale, and an unpinned dependency in the *published*
// package would let the two drift apart after release.
//
// Nothing here publishes, and nothing here builds a distribution: it only writes
// a package directory and prints it, so `uv build` (release.yml) or a test can
// take it from there.
//
// Usage: node scripts/python-sdk-build.mjs [--version <v>] [--out <dir>]

import { cpSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const REPO_ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const SOURCE = join(REPO_ROOT, "python", "notignored-sdk");

// The placeholders the committed manifest carries, spelled here so a change to
// either one fails loudly instead of publishing an unstamped package.
const VERSION_PLACEHOLDER = 'version = "0.0.0.dev0"';
const CLI_PLACEHOLDER = 'dependencies = ["notignored-cli"]';

// PEP 440 is wider than this, but the only versions this pipeline produces are
// the ones release-plz writes into Cargo.toml. Validating here means a malformed
// one fails in the build job rather than at the registry.
const VERSION = /^\d+\.\d+\.\d+(?:[-.]?[0-9A-Za-z.]+)?$/;

// Every failure names what to do next: this runs inside a release job, where the
// only diagnosis anyone gets is what it printed.
function die(msg, action) {
  process.stderr.write(`python-sdk-build: ${msg}\nACTION: ${action}\n`);
  process.exit(1);
}

// Run a filesystem step, turning anything it throws into this script's own
// diagnostic — Node's raw `ENOENT ... open '...'` names the syscall, not the fix.
function attempt(what, action, step) {
  try {
    return step();
  } catch (error) {
    die(`${what}: ${error.message}`, action);
  }
}

// Read the crate version from the root Cargo.toml [package] section. A tiny hand
// parser avoids a TOML dependency: take the first `version = "..."` after the
// `[package]` header and before the next section, so a dependency's version can
// never be mistaken for the crate's.
function cargoVersion() {
  const toml = attempt(
    "cannot read Cargo.toml",
    "run this from a checkout of the repository, where Cargo.toml is readable",
    () => readFileSync(join(REPO_ROOT, "Cargo.toml"), "utf8")
  );
  const pkg = toml.indexOf("[package]");
  if (pkg === -1) {
    die("no [package] section in Cargo.toml", "run this from the repository root");
  }
  const rest = toml.slice(pkg);
  const end = rest.indexOf("\n[", 1);
  const section = end === -1 ? rest : rest.slice(0, end);
  const m = section.match(/^\s*version\s*=\s*"([^"]+)"/m);
  if (!m) {
    die(
      "could not parse version from Cargo.toml [package]",
      'restore the `version = "X.Y.Z"` line release-plz maintains there'
    );
  }
  return m[1];
}

// Options are allowlisted: an unrecognized flag is a caller that meant something
// this script will not do, and ignoring it would assemble the wrong package.
function parseArgs(argv, allowed) {
  const out = {};
  const usage = allowed.map((name) => `--${name}`).join(", ");
  for (let i = 0; i < argv.length; i += 1) {
    const a = argv[i];
    if (!a.startsWith("--")) die(`unexpected argument: ${a}`, `pass options as ${usage}`);
    const key = a.slice(2);
    if (!allowed.includes(key)) die(`unknown option --${key}`, `this script takes ${usage}`);
    const val = argv[i + 1];
    if (val === undefined || val.startsWith("--")) {
      die(`--${key} needs a value`, `give --${key} a value`);
    }
    out[key] = val;
    i += 1;
  }
  return out;
}

const args = parseArgs(process.argv.slice(2), ["version", "out"]);
const version = args.version ?? cargoVersion();
if (!VERSION.test(version)) {
  die(
    `'${version}' is not a version PyPI can index`,
    args.version === undefined
      ? "fix the `version` in Cargo.toml [package]; it must read X.Y.Z"
      : "pass --version X.Y.Z, or omit it to take Cargo.toml's"
  );
}

const outRoot = resolve(args.out || join(REPO_ROOT, "python", "dist"));
const dest = join(outRoot, "notignored-sdk");

attempt(
  `cannot assemble the SDK package under ${outRoot}`,
  "check that --out names a writable directory",
  () => {
    rmSync(dest, { recursive: true, force: true });
    mkdirSync(dest, { recursive: true });
    // The wheel's payload: the importable package, its README, and its licence.
    // The tests stay out — they drive a binary the package does not carry.
    cpSync(join(SOURCE, "src"), join(dest, "src"), { recursive: true });
    cpSync(join(SOURCE, "README.md"), join(dest, "README.md"));
    cpSync(join(SOURCE, "LICENSE"), join(dest, "LICENSE"));
  }
);

const manifestPath = join(SOURCE, "pyproject.toml");
let manifest = attempt(
  `cannot read ${manifestPath}`,
  "restore python/notignored-sdk/pyproject.toml from git",
  () => readFileSync(manifestPath, "utf8")
);
for (const placeholder of [VERSION_PLACEHOLDER, CLI_PLACEHOLDER]) {
  if (!manifest.includes(placeholder)) {
    die(
      `python/notignored-sdk/pyproject.toml no longer contains \`${placeholder}\``,
      "restore that placeholder, or update this script deliberately — an unstamped " +
        "package would publish a dev version, or an unpinned notignored-cli"
    );
  }
}
manifest = manifest
  .replace(VERSION_PLACEHOLDER, `version = "${version}"`)
  .replace(CLI_PLACEHOLDER, `dependencies = ["notignored-cli==${version}"]`);

attempt(`cannot write ${join(dest, "pyproject.toml")}`, "check that --out is writable", () =>
  writeFileSync(join(dest, "pyproject.toml"), manifest)
);

process.stdout.write(`${dest}\n`);
