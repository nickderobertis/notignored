/**
 * The words this SDK will read, held against the words the CLI writes.
 *
 * `Tool`, `Scope` and `Change` are three vocabularies restated in three
 * languages — Rust in `src/model.rs`, Python in the PyPI SDK, TypeScript here —
 * and nothing generates one from another. What keeps them from drifting is this
 * gate: every refusal names the words this SDK knows, and each of those lists is
 * held against the crate's own, exhaustively, so a variant added on either side
 * fails until both sides have it.
 *
 * It is driven through `scan` — the surface a consumer has — against a
 * `notignored` from a build past this SDK, because a word outside the contract
 * is exactly what the real binary can never print.
 * `python/notignored-sdk/tests/test_vocabulary.py` holds the other side of the
 * same gate.
 *
 * Unix only, for the reason `resolution.test.mjs` gives: the stand-in command is
 * a shebang script.
 */

import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { test } from "node:test";

import { NotignoredContractError, scan } from "../dist/index.js";
import { notNotignored, REPO_ROOT, scratch } from "./support.mjs";

const unix = { skip: process.platform === "win32" ? "POSIX-only journeys" : false };

/**
 * Every word the crate spells for `enum`, read from its one source.
 *
 * `src/model.rs` maps each variant to the word that appears in reports, and that
 * match is exhaustive — a variant added without a word does not compile — so
 * these are all of them.
 */
function crateVocabulary(enumName) {
  const model = readFileSync(join(REPO_ROOT, "src", "model.rs"), "utf8");
  const words = [
    ...model.matchAll(new RegExp(`^\\s*${enumName}::\\w+ => "([^"]+)",$`, "gm")),
  ].map((match) => match[1]);
  assert.ok(
    words.length > 0,
    `no ${enumName} words in src/model.rs; the reader stopped seeing them`,
  );
  return words;
}

/** Run a scan against a build whose report names `mode`'s out-of-contract word. */
async function refusal(t, mode) {
  const override = process.env.NOTIGNORED_BIN;
  t.after(() => {
    if (override === undefined) delete process.env.NOTIGNORED_BIN;
    else process.env.NOTIGNORED_BIN = override;
  });
  process.env.NOTIGNORED_BIN = notNotignored(t, mode);

  let message;
  await assert.rejects(scan(["."], { cwd: scratch(t, `vocabulary-${mode}`) }), (error) => {
    assert.ok(
      error instanceof NotignoredContractError,
      `${error.name} is not a contract error`,
    );
    message = error.message;
    return true;
  });
  return message;
}

for (const [enumName, mode] of [
  ["Tool", "bad-tool"],
  ["Scope", "bad-scope"],
  ["Change", "bad-change"],
]) {
  test(`this SDK's ${enumName} vocabulary is the crate's own`, unix, async (t) => {
    const message = await refusal(t, mode);
    const named = message.match(/should be one of (.+?), got /);
    assert.ok(named, `the refusal does not name what it would accept: ${message}`);

    // The refusal names the words it will accept, so the list a consumer reads
    // is the list the reader enforces — and it has to be the crate's, word for
    // word and in order. A word the crate added and this SDK has not is a report
    // a user would be refused; one this SDK has and the crate does not is a
    // promise nothing can keep.
    assert.deepEqual(
      named[1].split(", "),
      crateVocabulary(enumName),
      `the ${enumName} vocabulary drifted from src/model.rs`,
    );
  });
}
