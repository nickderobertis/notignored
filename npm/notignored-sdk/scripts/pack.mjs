#!/usr/bin/env node
// Assemble the publishable `notignored-sdk` package: the compiled `dist`, the
// README, and a manifest stamped with the version from Cargo.toml.
//
// The committed manifest carries `0.0.0-managed` for the same reason
// npm/notignored's does — Cargo.toml is the repository's single version source
// (see the root AGENTS.md), and a real number here would be a second one to
// drift. release-plz bumps Cargo.toml; this reads it.
//
// Nothing here publishes and nothing here compiles: run the project's `build`
// target first. release.yml packs the directory this prints and hands the
// tarball to scripts/publish-npm.sh, exactly as it does for the launcher.
//
// Usage: node scripts/pack.mjs [--out <dir>]
// Prints the created package directory on stdout, and nothing else on success.

import { cpSync, existsSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const PROJECT = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const REPO_ROOT = resolve(PROJECT, "..", "..");

// This runs inside a release job, where what it printed is the only diagnosis
// anyone gets — so every failure names the next action.
function die(message, action) {
  process.stderr.write(`sdk-pack: ${message}\nACTION: ${action}\n`);
  process.exit(1);
}

function attempt(what, action, step) {
  try {
    return step();
  } catch (error) {
    die(`${what}: ${error.message}`, action);
  }
}

// npm rejects anything that is not semver, and a version with a stray specifier
// would publish under a name no consumer could ask for.
const VERSION = /^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?$/;

// The crate version, from the root Cargo.toml [package] section. A hand parser
// rather than a TOML dependency, and scoped to that section so a dependency's
// version can never be mistaken for the crate's — the same reading
// scripts/npm-build.mjs does for the launcher.
function cargoVersion() {
  const toml = attempt(
    "cannot read Cargo.toml",
    "run this from a checkout of the repository, where Cargo.toml is readable",
    () => readFileSync(join(REPO_ROOT, "Cargo.toml"), "utf8"),
  );
  const start = toml.indexOf("[package]");
  if (start === -1) die("no [package] section in Cargo.toml", "run this from a full checkout");
  const rest = toml.slice(start);
  const end = rest.indexOf("\n[", 1);
  const section = end === -1 ? rest : rest.slice(0, end);
  const found = section.match(/^\s*version\s*=\s*"([^"]+)"/m);
  if (!found) {
    die(
      "could not parse version from Cargo.toml [package]",
      'restore the `version = "X.Y.Z"` line release-plz maintains there',
    );
  }
  if (!VERSION.test(found[1])) {
    die(
      `'${found[1]}' is not a version npm can index`,
      "fix the `version` in Cargo.toml [package]; it must read X.Y.Z",
    );
  }
  return found[1];
}

function parseArgs(argv) {
  const out = {};
  for (let i = 0; i < argv.length; i += 1) {
    if (argv[i] !== "--out") die(`unknown option ${argv[i]}`, "this script takes --out <dir>");
    const value = argv[i + 1];
    if (value === undefined || value.startsWith("--"))
      die("--out needs a value", "give --out a directory");
    out.out = value;
    i += 1;
  }
  return out;
}

const args = parseArgs(process.argv.slice(2));
const version = cargoVersion();
const outRoot = resolve(args.out ?? join(REPO_ROOT, "npm", "dist"));
const dest = join(outRoot, "notignored-sdk");

const built = join(PROJECT, "dist");
if (!existsSync(join(built, "index.js"))) {
  die(`nothing compiled at ${built}`, "build it first: just nx run notignored-sdk-npm:build");
}

attempt(
  `cannot assemble the package under ${outRoot}`,
  "check that --out names a writable directory",
  () => {
    rmSync(dest, { recursive: true, force: true });
    mkdirSync(dest, { recursive: true });
    cpSync(built, join(dest, "dist"), { recursive: true });
    cpSync(join(PROJECT, "README.md"), join(dest, "README.md"));
  },
);

const manifest = attempt(
  "the committed manifest is missing or is not JSON",
  "restore npm/notignored-sdk/package.json from git",
  () => JSON.parse(readFileSync(join(PROJECT, "package.json"), "utf8")),
);
manifest.version = version;
attempt(`cannot write ${dest}/package.json`, "check that --out is writable", () =>
  writeFileSync(join(dest, "package.json"), `${JSON.stringify(manifest, null, 2)}\n`),
);

process.stdout.write(`${dest}\n`);
