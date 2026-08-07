/**
 * `--diff`, over a git repository this suite builds commit by commit.
 *
 * The review case the product exists for: a pull request adds a suppression,
 * and only that one should be reported. Proving it needs real history — a
 * base commit, a change on top — because the semantics are git's (three-dot,
 * added lines only), not something the SDK could stand in for.
 */

import assert from "node:assert/strict";
import { test } from "node:test";

import { NotignoredUsageError, scan } from "../dist/index.js";
import { binary, commit, file, git, gitRepo, scratch } from "./support.mjs";

const BIN = binary();

const BASE = "import os  # noqa: F401  # already reviewed\n";
const ADDED = "import sys  # noqa: F811  # newly added\n";

test("a diff scan reports only what the working tree added", async (t) => {
  const dir = gitRepo(t, "diff-worktree");
  file(dir, "app.py", BASE);
  commit(dir, "base");
  file(dir, "app.py", BASE + ADDED);

  const report = await scan(["."], { cwd: dir, bin: BIN, diff: true });

  assert.deepEqual(
    report.ignores.map((directive) => [directive.line, directive.rules]),
    [[2, ["F811"]]],
    "the suppression already in HEAD is not this change's",
  );
});

test("a diff scan of an unchanged tree is empty", async (t) => {
  const dir = gitRepo(t, "diff-clean");
  file(dir, "app.py", BASE);
  commit(dir, "base");

  const report = await scan(["."], { cwd: dir, bin: BIN, diff: true });

  assert.deepEqual(report.ignores, []);
  assert.deepEqual(report.errors, []);
});

test("diffBase compares against the branch point, not the base branch's later commits", async (t) => {
  const dir = gitRepo(t, "diff-base");
  file(dir, "app.py", BASE);
  commit(dir, "base");

  git(dir, "switch", "--quiet", "--create", "feature");
  file(dir, "app.py", BASE + ADDED);
  commit(dir, "add a suppression");

  // A commit that lands on main *after* the branch forked. Three-dot semantics
  // mean it is not this branch's change, so its suppression must not be
  // reported — a two-dot comparison would blame this branch for it.
  git(dir, "switch", "--quiet", "main");
  file(dir, "other.py", "import json  # noqa: F401  # someone else's\n");
  commit(dir, "unrelated work on main");
  git(dir, "switch", "--quiet", "feature");

  const report = await scan(["."], {
    cwd: dir,
    bin: BIN,
    diff: true,
    diffBase: "main",
  });

  assert.deepEqual(
    report.ignores.map((directive) => [directive.path, directive.rules]),
    [["app.py", ["F811"]]],
  );
});

test("paths narrow a diff scan the same way they narrow a tree scan", async (t) => {
  const dir = gitRepo(t, "diff-paths");
  file(dir, "app.py", BASE);
  file(dir, "other.py", BASE);
  commit(dir, "base");
  file(dir, "app.py", BASE + ADDED);
  file(dir, "other.py", BASE + ADDED);

  const report = await scan(["other.py"], { cwd: dir, bin: BIN, diff: true });

  assert.deepEqual(
    report.ignores.map((directive) => directive.path),
    ["other.py"],
  );
});

test("a diff outside a repository rejects with what git said", async (t) => {
  const dir = scratch(t, "diff-none");
  file(dir, "app.py", BASE);

  await assert.rejects(scan(["."], { cwd: dir, bin: BIN, diff: true }), (error) => {
    assert.equal(error.exitCode, 2);
    assert.match(error.stderr, /cannot diff/);
    return true;
  });
});

/**
 * The CLI's `--diff-base` carries `requires = "diff"`, so clap refuses the pair
 * at the boundary. The SDK refuses it at its own boundary instead of spending a
 * process to be told the same thing.
 */
test("diffBase without diff is refused before anything is spawned", async () => {
  // No `bin`, and none of these journeys install one: whether a call describes
  // a run is decided before the environment is consulted at all.
  await assert.rejects(scan(["."], { diffBase: "main" }), (error) => {
    assert.ok(error instanceof NotignoredUsageError, `${error.name} is not a usage error`);
    assert.match(error.message, /diffBase requires diff: true/);
    return true;
  });
});

test("an empty diffBase is refused too", async () => {
  await assert.rejects(
    scan(["."], { diff: true, diffBase: "", bin: BIN }),
    NotignoredUsageError,
  );
});
