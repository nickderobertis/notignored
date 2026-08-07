/**
 * The placeholder tier: it proves the wiring, not an SDK surface.
 *
 * Until the SDK lands, what can go wrong here is the plumbing — an `exports` map
 * that resolves nowhere, a `test` target Nx runs from the wrong directory, a
 * manifest whose name stopped matching what it publishes. Importing the package
 * through its own entry point fails on all three.
 */

import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";

import { packageName } from "../src/index.mjs";

test("the package resolves through its own exports map", async () => {
  const entry = await import("../src/index.mjs");
  assert.equal(entry.packageName, packageName);
});

test("the manifest publishes under the name the entry point reports", () => {
  const manifest = JSON.parse(
    readFileSync(new URL("../package.json", import.meta.url), "utf8"),
  );
  assert.equal(manifest.name, packageName);
});
