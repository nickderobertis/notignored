import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { test } from "node:test";

// The provider probes call this CLI; the classifier and packagers import the
// same validators, so one journey over the CLI covers the shared grammar.
function validate(kind, value) {
  return spawnSync(
    "node",
    [
      "scripts/release-version.mjs",
      "validate",
      "--kind",
      kind,
      "--value",
      value,
    ],
    { encoding: "utf8" },
  );
}

test("release and Python metadata versions follow their published grammars", () => {
  for (const [kind, value, valid] of [
    ["release", "0.1.12", true],
    ["release", "1.2.3-rc.1+build.5", true],
    ["release", "0.0.0-managed", true],
    ["release", "01.2.3", false],
    ["release", "1.2.3-..", false],
    ["release", "1.2.3+one+two", false],
    ["release", "1.2.3\n", false],
    ["tag", "v0.1.12", true],
    ["tag", "0.1.12", false],
    ["python", "0.1.12", true],
    ["python", "0.0.0.dev0", true],
    ["python", "1.2.3rc1", true],
    ["python", "1.2.3.foo", false],
    ["python", "1.2.3-rc.1", false],
    ["unknown", "0.1.12", false],
  ]) {
    const result = validate(kind, value);
    assert.equal(
      result.status,
      valid ? 0 : 1,
      `${kind} ${JSON.stringify(value)}`,
    );
    if (!valid) assert.match(result.stderr, /ACTION:/);
  }
});
