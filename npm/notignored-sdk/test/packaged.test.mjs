/**
 * The package a release publishes, built and installed for real.
 *
 * Everything else in this suite imports `../dist`. That proves the code and
 * says nothing about what npm would serve — a `files` list that forgot `dist`,
 * an `exports` map pointing at a path the tarball does not carry, or a version
 * that stopped coming from Cargo.toml all survive it. So this journey packs the
 * package, installs it into a scratch project beside the real `notignored-cli`
 * launcher, and scans a fixture through the entry point a consumer imports.
 *
 * It is also the one place the launcher branch of binary resolution is real:
 * nothing here sets `NOTIGNORED_BIN` or leaves anything on `PATH`, so the only
 * way the scan can find a binary is the sibling package npm installed.
 */

import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { cpSync, mkdirSync, readFileSync, readdirSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { test } from "node:test";

import { PROJECT, REPO_ROOT, binary, file, run, scratch } from "./support.mjs";

/** The Rust target triple for this host, as release.yml's matrices spell it. */
const HOST_TARGETS = {
  "linux-x64": "x86_64-unknown-linux-gnu",
  "linux-arm64": "aarch64-unknown-linux-gnu",
  "darwin-x64": "x86_64-apple-darwin",
  "darwin-arm64": "aarch64-apple-darwin",
  "win32-x64": "x86_64-pc-windows-msvc",
};

/** npm ships as a batch file on Windows, which only that spelling can spawn. */
const NPM = process.platform === "win32" ? "npm.cmd" : "npm";

/** The version every artifact in this repository takes, from its one source. */
function cargoVersion() {
  const toml = readFileSync(join(REPO_ROOT, "Cargo.toml"), "utf8");
  const section = toml.slice(toml.indexOf("[package]")).split("\n[")[0];
  const found = section.match(/^\s*version\s*=\s*"([^"]+)"/m);
  assert.ok(found, "Cargo.toml [package] declares a version");
  return found[1];
}

/** Assemble the SDK package with its own script, returning the directory. */
function packSdk(out) {
  return run("pack.mjs", process.execPath, ["scripts/pack.mjs", "--out", out], {
    cwd: PROJECT,
  }).trim();
}

/**
 * `npm pack` a package directory, returning the tarball.
 *
 * Each package packs into its own destination: the launcher and the platform
 * package share a name prefix, so a shared one makes "the tarball for this
 * package" ambiguous.
 */
function pack(source, into) {
  const name = JSON.parse(readFileSync(join(source, "package.json"), "utf8")).name;
  const destination = join(into, name);
  mkdirSync(destination, { recursive: true });
  run("npm pack", NPM, ["pack", "--pack-destination", destination], { cwd: source });
  const tarball = readdirSync(destination).find((entry) => entry.endsWith(".tgz"));
  assert.ok(tarball, `npm pack produced no tarball in ${destination}`);
  return join(destination, tarball);
}

test("the packed package is the SDK at the version Cargo.toml declares", (t) => {
  const packageDir = packSdk(join(scratch(t, "pack"), "dist"));

  const manifest = JSON.parse(readFileSync(join(packageDir, "package.json"), "utf8"));
  assert.equal(manifest.name, "notignored-sdk");
  assert.equal(manifest.version, cargoVersion());
  assert.notEqual(
    JSON.parse(readFileSync(join(PROJECT, "package.json"), "utf8")).version,
    manifest.version,
    "the committed manifest holds the placeholder, so it can never be a second version source",
  );
  assert.deepEqual(readdirSync(packageDir).sort(), ["README.md", "dist", "package.json"]);
});

test("pack refuses to assemble a package with nothing compiled in it", (t) => {
  // A checkout-shaped tree: a Cargo.toml to read the version from, and the
  // project laid out where the script expects to find itself — but no `dist`.
  const root = scratch(t, "unbuilt");
  const project = join(root, "npm", "notignored-sdk");
  mkdirSync(join(project, "scripts"), { recursive: true });
  writeFileSync(
    join(root, "Cargo.toml"),
    '[package]\nname = "notignored"\nversion = "9.9.9"\n',
  );
  cpSync(join(PROJECT, "scripts", "pack.mjs"), join(project, "scripts", "pack.mjs"));
  cpSync(join(PROJECT, "package.json"), join(project, "package.json"));

  const attempt = spawnSync(
    process.execPath,
    ["scripts/pack.mjs", "--out", join(root, "out")],
    {
      cwd: project,
      encoding: "utf8",
    },
  );

  assert.equal(attempt.status, 1);
  assert.match(attempt.stderr, /nothing compiled at/);
  assert.match(attempt.stderr, /ACTION: build it first/);
});

test("installed from its tarball, the SDK scans through the launcher npm resolved", (t) => {
  const work = scratch(t, "install");
  const dist = join(work, "dist");
  const tarballs = join(work, "tarballs");
  const target = HOST_TARGETS[`${process.platform}-${process.arch}`];
  assert.ok(
    target,
    `no released target for ${process.platform}-${process.arch}\n` +
      "ACTION: add it to release.yml's matrices, scripts/npm-build.mjs, and HOST_TARGETS here",
  );

  const npmBuild = (...args) =>
    run("npm-build.mjs", process.execPath, ["scripts/npm-build.mjs", ...args], {
      cwd: REPO_ROOT,
    }).trim();
  const platform = npmBuild(
    "platform",
    "--target",
    target,
    "--binary",
    binary(),
    "--out",
    dist,
  );
  const launcher = npmBuild("launcher", "--out", dist);
  const sdk = packSdk(dist);

  const project = join(work, "consumer");
  mkdirSync(project);
  writeFileSync(
    join(project, "package.json"),
    `${JSON.stringify({ name: "consumer", version: "1.0.0", private: true, type: "module" }, null, 2)}\n`,
  );
  // Offline keeps the gate hermetic: every argument is a local tarball, so a
  // step that reached for the registry would be a bug rather than latency.
  run(
    "npm install",
    NPM,
    [
      "install",
      "--offline",
      "--no-audit",
      "--no-fund",
      pack(platform, tarballs),
      pack(launcher, tarballs),
      pack(sdk, tarballs),
    ],
    { cwd: project },
  );

  file(project, "app.py", "import os  # noqa: F401  # re-exported\n");

  // No override, and nothing on PATH: the only binary this can find is the one
  // npm installed as a sibling of the SDK.
  const environment = { ...process.env, PATH: "", Path: "" };
  delete environment.NOTIGNORED_BIN;
  const consumer = spawnSync(
    process.execPath,
    ["--input-type=module", "-e", CONSUMER_PROGRAM],
    {
      cwd: project,
      encoding: "utf8",
      env: environment,
    },
  );

  assert.equal(consumer.status, 0, `${consumer.stdout}\n${consumer.stderr}`);
  assert.deepEqual(JSON.parse(consumer.stdout), {
    version: cargoVersion(),
    ignores: [["app.py", "ruff", ["F401"], "re-exported"]],
  });
});

/** What the installed package is asked to do, as a consumer would write it. */
const CONSUMER_PROGRAM = `
import { readFileSync } from "node:fs";
import { scan } from "notignored-sdk";

const report = await scan(["app.py"]);
const installed = JSON.parse(
  readFileSync("node_modules/notignored-sdk/package.json", "utf8"),
);
process.stdout.write(
  JSON.stringify({
    version: installed.version,
    ignores: report.ignores.map((d) => [d.path, d.tool, d.rules, d.reason]),
  }),
);
`;
