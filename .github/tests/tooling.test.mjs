import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { test } from "node:test";

const tooling = resolve(".github/tooling-check.mjs");

// Each case is a tree the wrapper must refuse before any formatter runs, so
// the real wrapper is driven over a scratch repository root.
function refusal(mode, files) {
  const root = mkdtempSync(
    join(process.env.ONEPIPELINE_NODE_SCRATCH_DIR ?? tmpdir(), "tooling-"),
  );
  try {
    mkdirSync(join(root, ".github/workflows"), { recursive: true });
    for (const [path, text] of Object.entries(files))
      writeFileSync(join(root, ".github", path), text);
    return spawnSync("node", [tooling, mode], { cwd: root, encoding: "utf8" });
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
}

const validCi =
  "jobs:\n  required:\n    runs-on: ubuntu-latest\n    needs: []\n";

test("the tooling wrapper refuses drift, malformed files and invalid job shapes with an action", () => {
  for (const [mode, files, message] of [
    ["unknown", {}, /unknown tooling check/],
    ["format-check", { "data.json": '{"a":1}\n' }, /JSON formatting drift/],
    ["lint", { "broken.mjs": "export const = ;\n" }, /broken\.mjs/],
    ["shape-check", { "dup.yml": "a: 1\na: 2\n" }, /Map keys must be unique/],
    [
      "shape-check",
      {
        "workflows/ci.yml":
          "jobs:\n  required:\n    runs-on: ubuntu-latest\n    needs: lint\n",
      },
      /required\.needs must be a list/,
    ],
    [
      "shape-check",
      { "workflows/ci.yml": `${validCi}  shapeless:\n    steps: []\n` },
      /invalid job type: shapeless/,
    ],
  ]) {
    const result = refusal(mode, files);
    assert.equal(result.status, 1, `${mode}: ${result.stdout}`);
    assert.match(result.stderr, message);
    assert.match(result.stderr, /ACTION: fix the named syntax\/shape/);
  }
  const accepted = refusal("shape-check", { "workflows/ci.yml": validCi });
  assert.equal(accepted.status, 0, accepted.stderr);
  assert.match(accepted.stdout, /shape-check passed/);
});
