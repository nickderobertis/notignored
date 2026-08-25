/**
 * The tree scan: what a caller gets back, over the real binary and real files.
 *
 * These assert on the *record*, field by field, rather than on a count. The
 * contract is the product — a scan that found the right number of directives
 * and mislabelled their scope is still wrong — and the fields are what a
 * reviewer reads.
 *
 * `scan` takes no binary argument, so the binary is named the one way the
 * public surface allows: `NOTIGNORED_BIN`. `node --test` runs each file in its
 * own process, so setting it here reaches these journeys and nothing else.
 */

import assert from "node:assert/strict";
import { test } from "node:test";

import { NotignoredExitError, NotignoredUsageError, scan } from "../dist/index.js";
import { binary, file, scratch } from "./support.mjs";

process.env.NOTIGNORED_BIN = binary();

/** A tree with one suppression per language, so a filter has something to cut. */
function polyglot(t) {
  const dir = scratch(t, "scan");
  file(dir, "app.py", "import os  # noqa: F401  # re-exported\nx = 1\n");
  file(
    dir,
    "app.ts",
    "// biome-ignore lint/suspicious/noExplicitAny: third-party shape\nconst a: any = 1;\nexport default a;\n",
  );
  file(dir, "lib.rs", "#[allow(dead_code)]\nfn unused() {}\n");
  return dir;
}

test("a folder scan returns every suppression in the tree", async (t) => {
  const dir = polyglot(t);

  const report = await scan(["."], { cwd: dir });

  assert.equal(report.version, 1);
  assert.deepEqual(report.errors, []);
  assert.deepEqual(
    report.ignores.map((directive) => [directive.path, directive.tool]),
    [
      ["app.py", "ruff"],
      ["app.ts", "biome"],
      ["lib.rs", "rust"],
    ],
    "the report is ordered by path, and one tool claimed each file",
  );

  const [noqa] = report.ignores;
  assert.deepEqual(noqa, {
    tool: "ruff",
    scope: "line",
    rules: ["F401"],
    reason: "re-exported",
    path: "app.py",
    line: 1,
    end_line: 1,
    column: 12,
    raw: "# noqa: F401  # re-exported",
    suppressed: { start_line: 1, end_line: 1 },
    // A tree scan has no base, so it says nothing about what changed.
    change: null,
  });
});

test("omitting the paths scans the working directory, as the CLI does", async (t) => {
  const dir = polyglot(t);

  const report = await scan(undefined, { cwd: dir });

  assert.deepEqual(report.ignores.map((directive) => directive.path).sort(), [
    "app.py",
    "app.ts",
    "lib.rs",
  ]);
});

test("a single file narrows the scan to that file", async (t) => {
  const dir = polyglot(t);

  const report = await scan(["app.py"], { cwd: dir });

  assert.deepEqual(
    report.ignores.map((directive) => directive.path),
    ["app.py"],
  );
});

test("the tools filter reports only the tools it names", async (t) => {
  const dir = polyglot(t);

  const report = await scan(["."], { cwd: dir, tools: ["ruff", "rust"] });

  assert.deepEqual(
    report.ignores.map((directive) => directive.tool),
    ["ruff", "rust"],
  );
});

test("an empty tools filter is not a filter at all", async (t) => {
  const dir = polyglot(t);

  const report = await scan(["."], { cwd: dir, tools: [] });

  assert.equal(report.ignores.length, 3);
});

/**
 * A blanket, reason-less directive: `rules` empty and `reason` null are the two
 * places the contract uses "nothing here", and a reader that dropped either
 * would report an unjustified suppression as justified.
 */
test("a blanket suppression reports empty rules and a null reason", async (t) => {
  const dir = scratch(t, "blanket");
  file(dir, "app.py", "y = 2  # noqa\n");

  const report = await scan(["."], { cwd: dir });

  assert.deepEqual(
    report.ignores.map((d) => [d.rules, d.reason]),
    [[[], null]],
  );
});

/**
 * The real report-error case, end to end: `notignored` reports a file it could
 * not read *both* ways — an `errors` entry on stdout and exit 2 — and only the
 * exit is the verdict on whether the scan completed. Resolving the stdout half
 * would hand a caller a report they would read as "this tree is clean", from a
 * run that never opened one of its files.
 */
test("an unreadable file is a non-zero exit, not a report", async (t) => {
  const dir = scratch(t, "unreadable");
  file(dir, "app.py", "import os  # noqa: F401  # re-exported\n");
  const locked = file(dir, "locked.py", "y = 2  # noqa\n");
  // The scratch tree is removed whatever happens, and removing a file needs
  // write access to its directory rather than to the file — so nothing has to
  // put these permissions back.
  const { chmodSync, readFileSync } = await import("node:fs");
  chmodSync(locked, 0o000);
  assert.throws(
    () => readFileSync(locked),
    "the scratch file is still readable, so this journey would prove nothing\n" +
      "ACTION: run the suite as a user that is not root",
  );

  await assert.rejects(scan(["."], { cwd: dir }), (error) => {
    assert.ok(error instanceof NotignoredExitError, `${error.name} is not an exit error`);
    assert.equal(error.exitCode, 2);
    assert.equal(error.signal, null);
    // Verbatim: the binary already names the file and the reason, and this is
    // the only place a caller can read them.
    assert.match(error.stderr, /locked\.py/);
    assert.match(error.stderr, /[Pp]ermission denied/);
    return true;
  });
});

test("a path that does not exist is a non-zero exit carrying the hint", async (t) => {
  const dir = scratch(t, "missing-path");

  await assert.rejects(scan(["nope"], { cwd: dir }), (error) => {
    assert.ok(error instanceof NotignoredExitError, `${error.name} is not an exit error`);
    assert.equal(error.exitCode, 2);
    assert.match(error.stderr, /no such file or directory/);
    assert.match(error.stderr, /hint: pass paths that exist/);
    return true;
  });
});

test("a path is data, never a flag", async (t) => {
  const dir = scratch(t, "dashes");
  file(dir, "app.py", "y = 2  # noqa\n");

  // Without the `--` separator the CLI would read this as an option and exit 2;
  // with it, it is simply a path that does not exist.
  await assert.rejects(scan(["--fail-if-found"], { cwd: dir }), (error) => {
    assert.match(error.stderr, /--fail-if-found/);
    assert.match(error.stderr, /no such file or directory/);
    return true;
  });
});

test("an empty path is refused before anything is spawned", async () => {
  await assert.rejects(scan([""]), NotignoredUsageError);
});

test("paths must be an array", async () => {
  await assert.rejects(scan("src"), NotignoredUsageError);
});
