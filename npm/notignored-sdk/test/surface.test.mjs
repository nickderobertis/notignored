/**
 * The public surface, held to exactly what was approved.
 *
 * `scan`, the six record types, the six error classes — and nothing else. The
 * options bag is not among them, and neither is any way to name the binary from
 * a call site: that is `NOTIGNORED_BIN`'s job. None of this is visible to the
 * suites that *use* the SDK, because using an API cannot show what it also
 * offers; so the compiled entry point is read directly, as a consumer's
 * bundler and editor read it.
 */

import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { test } from "node:test";

import { PROJECT } from "./support.mjs";

/** Everything the entry point exports at runtime. Types leave nothing behind. */
const RUNTIME_EXPORTS = [
  "NotignoredBinaryNotFoundError",
  "NotignoredContractError",
  "NotignoredError",
  "NotignoredExitError",
  "NotignoredSpawnError",
  "NotignoredUsageError",
  "scan",
];

/** The types a consumer may import. */
const TYPE_EXPORTS = [
  "Change",
  "IgnoreDirective",
  "Report",
  "ReportError",
  "Scope",
  "Suppressed",
  "Tool",
];

function declarations() {
  return readFileSync(join(PROJECT, "dist", "index.d.ts"), "utf8");
}

test("the entry point exports exactly the approved runtime names", async () => {
  const entry = await import("../dist/index.js");

  assert.deepEqual(
    Object.keys(entry).sort(),
    RUNTIME_EXPORTS,
    "the runtime surface drifted from the approved API",
  );
});

test("the declarations export exactly the approved types", () => {
  const exported = [...declarations().matchAll(/^export (?:type )?\{([^}]*)\}/gm)]
    .flatMap((match) => match[1].split(","))
    .map((name) => name.trim())
    .filter((name) => name !== "");
  // Every form that adds a name, not just the ones used today: a surface test
  // that only knew about `export declare function` would miss the first
  // `export interface` somebody added.
  const declared = [
    ...declarations().matchAll(
      /^export (?:declare )?(?:function|class|const|interface|type|enum) (\w+)/gm,
    ),
  ].map((match) => match[1]);

  assert.deepEqual(
    [...exported, ...declared].sort(),
    [...TYPE_EXPORTS, ...RUNTIME_EXPORTS].sort(),
    "the declared surface drifted from the approved API",
  );
});

/**
 * The options bag is a parameter type, not a product. Exporting it would let a
 * consumer build against a name the contract never promised, and the first
 * option added or removed would be a breaking change to something nobody meant
 * to publish.
 */
test("the options type is not exported", () => {
  const text = declarations();

  assert.match(text, /^interface ScanOptions \{/m, "the options type still describes the call");
  // Referring to it from `scan`'s signature is unavoidable and harmless; being
  // *declared* exported, or named in an export list, is what makes it
  // importable.
  assert.doesNotMatch(
    text,
    /^export (?:declare )?(?:interface|type) ScanOptions\b/m,
    "ScanOptions is declared exported; the approved surface is scan, the records, and the errors",
  );
  assert.doesNotMatch(
    text,
    /^export (?:type )?\{[^}]*\bScanOptions\b/m,
    "ScanOptions is named in an export list; it is a parameter type, not a product",
  );
});

/**
 * The approved signature, spelled out — not merely the set of names in it.
 *
 * A type can drift wider without any name changing. `readonly string[]` accepts
 * everything `string[]` does and more, and `tools?: Tool[] | undefined` accepts
 * an explicit `undefined` the contract never promised; both would pass a check
 * that only counted fields. Widening a published surface is as much a change to
 * it as narrowing one, so this asserts the text a consumer's editor shows.
 *
 * Which binary runs is not among these options and must not become one: a `bin`
 * argument would be a second mechanism to document, to validate, and to keep in
 * step with `NOTIGNORED_BIN`.
 */
test("scan declares exactly the approved signature", () => {
  const text = declarations();

  const signature = text.match(/^export declare function scan\((.*)\): Promise<Report>;$/m);
  assert.ok(signature, "the entry point still declares scan");
  assert.equal(signature[1], "paths?: string[], options?: ScanOptions");

  const options = text.match(/^interface ScanOptions \{([\s\S]*?)^\}/m);
  assert.ok(options, "the options type is still declared");
  const fields = [...options[1].matchAll(/^ {4}(\w+)(\??): (.+);$/gm)].map((match) => [
    `${match[1]}${match[2]}`,
    match[3],
  ]);
  assert.deepEqual(fields.sort(), [
    ["cwd?", "string"],
    ["diff?", "boolean"],
    ["diffBase?", "string"],
    ["tools?", "Tool[]"],
  ]);
});
