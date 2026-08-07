/**
 * The tree scan: what a caller gets back, over the real binary and real files.
 *
 * These assert on the *record*, field by field, rather than on a count. The
 * contract is the product — a scan that found the right number of directives
 * and mislabelled their scope is still wrong — and the fields are what a
 * reviewer reads.
 */

import assert from "node:assert/strict";
import { test } from "node:test";

import { NotignoredUsageError, scan } from "../dist/index.js";
import { binary, file, scratch } from "./support.mjs";

const BIN = binary();

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

  const report = await scan(["."], { cwd: dir, bin: BIN });

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
  });
});

test("omitting the paths scans the working directory, as the CLI does", async (t) => {
  const dir = polyglot(t);

  const report = await scan(undefined, { cwd: dir, bin: BIN });

  assert.deepEqual(report.ignores.map((directive) => directive.path).sort(), [
    "app.py",
    "app.ts",
    "lib.rs",
  ]);
});

test("a single file narrows the scan to that file", async (t) => {
  const dir = polyglot(t);

  const report = await scan(["app.py"], { cwd: dir, bin: BIN });

  assert.deepEqual(
    report.ignores.map((directive) => directive.path),
    ["app.py"],
  );
});

test("the tools filter reports only the tools it names", async (t) => {
  const dir = polyglot(t);

  const report = await scan(["."], { cwd: dir, bin: BIN, tools: ["ruff", "rust"] });

  assert.deepEqual(
    report.ignores.map((directive) => directive.tool),
    ["ruff", "rust"],
  );
});

test("an empty tools filter is not a filter at all", async (t) => {
  const dir = polyglot(t);

  const report = await scan(["."], { cwd: dir, bin: BIN, tools: [] });

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

  const report = await scan(["."], { cwd: dir, bin: BIN });

  assert.deepEqual(
    report.ignores.map((d) => [d.rules, d.reason]),
    [[[], null]],
  );
});

/**
 * A file the binary could not read is a report `error`, not an exception —
 * even though the process exits non-zero for it. Losing the whole scan to one
 * unreadable file would hide every suppression that *was* found, which for a
 * reviewer is the failure that matters.
 */
test("an unreadable file lands in report.errors and the rest of the scan survives", async (t) => {
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

  const report = await scan(["."], { cwd: dir, bin: BIN });

  assert.deepEqual(
    report.ignores.map((directive) => directive.path),
    ["app.py"],
    "the readable file was still reported",
  );
  assert.deepEqual(
    report.errors.map((error) => error.path),
    ["locked.py"],
  );
  assert.match(report.errors[0].message, /[Pp]ermission denied/);
});

test("a path is data, never a flag", async (t) => {
  const dir = scratch(t, "dashes");
  file(dir, "app.py", "y = 2  # noqa\n");

  // Without the `--` separator the CLI would read this as an option and exit 2;
  // with it, it is simply a path that does not exist.
  await assert.rejects(scan(["--fail-if-found"], { cwd: dir, bin: BIN }), (error) => {
    assert.match(error.stderr, /--fail-if-found/);
    assert.match(error.stderr, /no such file or directory/);
    return true;
  });
});

test("an empty path is refused before anything is spawned", async () => {
  await assert.rejects(scan([""], { bin: BIN }), NotignoredUsageError);
});

test("paths must be an array", async () => {
  await assert.rejects(scan("src"), NotignoredUsageError);
});
