/**
 * Which binary a scan runs, and every way that can go wrong.
 *
 * Binary resolution is the SDK's other trust boundary: everything downstream
 * assumes the command it spawned is `notignored`. These journeys drive each
 * source for real — an override, `PATH`, nothing at all — and each refusal
 * through a program that genuinely is not `notignored`.
 *
 * Unix only. The scratch commands are a symlink and a shebang script, and the
 * suite's own `PATH` manipulation has no Windows equivalent; the same reason
 * `tests/e2e/js_tools_setup.rs` is Unix-only.
 */

import assert from "node:assert/strict";
import { delimiter, join } from "node:path";
import { test } from "node:test";

import {
  NotignoredBinaryNotFoundError,
  NotignoredContractError,
  NotignoredExitError,
  NotignoredSpawnError,
  NotignoredUsageError,
  scan,
} from "../dist/index.js";
import { binary, binaryOnPath, file, notNotignored, scratch } from "./support.mjs";

const unix = { skip: process.platform === "win32" ? "POSIX-only journeys" : false };

const BIN = binary();
const FIXTURE = "import os  # noqa: F401  # re-exported\n";

/** Restore `PATH` and `NOTIGNORED_BIN` however the journey ends. */
function isolatedEnvironment(t) {
  const path = process.env.PATH;
  const override = process.env.NOTIGNORED_BIN;
  t.after(() => {
    process.env.PATH = path;
    if (override === undefined) delete process.env.NOTIGNORED_BIN;
    else process.env.NOTIGNORED_BIN = override;
  });
  delete process.env.NOTIGNORED_BIN;
}

function withFixture(t, label) {
  const dir = scratch(t, label);
  file(dir, "app.py", FIXTURE);
  return dir;
}

test("NOTIGNORED_BIN names the binary to run", unix, async (t) => {
  isolatedEnvironment(t);
  const dir = withFixture(t, "env");
  process.env.NOTIGNORED_BIN = BIN;

  const report = await scan(["."], { cwd: dir });

  assert.deepEqual(
    report.ignores.map((d) => d.rules),
    [["F401"]],
  );
});

test("without an override, the binary is found on PATH", unix, async (t) => {
  isolatedEnvironment(t);
  const dir = withFixture(t, "path-scan");
  process.env.PATH = binaryOnPath(t);

  const report = await scan(["."], { cwd: dir });

  assert.deepEqual(
    report.ignores.map((d) => d.rules),
    [["F401"]],
  );
});

test("PATH entries are tried in order", unix, async (t) => {
  isolatedEnvironment(t);
  const dir = withFixture(t, "path-order");
  const empty = scratch(t, "empty");
  process.env.PATH = [empty, binaryOnPath(t)].join(delimiter);

  const report = await scan(["."], { cwd: dir });

  assert.deepEqual(
    report.ignores.map((d) => d.rules),
    [["F401"]],
  );
});

test("a directory named notignored on PATH is not a binary", unix, async (t) => {
  isolatedEnvironment(t);
  const decoy = scratch(t, "decoy");
  const { mkdirSync } = await import("node:fs");
  mkdirSync(join(decoy, "notignored"));
  process.env.PATH = decoy;

  await assert.rejects(scan(["."], { cwd: decoy }), NotignoredBinaryNotFoundError);
});

test("with nothing installed, the error says how to install one", unix, async (t) => {
  isolatedEnvironment(t);
  const dir = withFixture(t, "nowhere");
  process.env.PATH = scratch(t, "empty-path");

  await assert.rejects(scan(["."], { cwd: dir }), (error) => {
    assert.ok(
      error instanceof NotignoredBinaryNotFoundError,
      `${error.name} is not a not-found error`,
    );
    for (const install of ["npm install notignored-cli", "pip install", "cargo install"]) {
      assert.ok(
        error.message.includes(install),
        `${error.message} does not mention ${install}`,
      );
    }
    assert.match(error.message, /NOTIGNORED_BIN/);
    return true;
  });
});

test("an empty NOTIGNORED_BIN is no override at all", unix, async (t) => {
  isolatedEnvironment(t);
  const dir = withFixture(t, "empty-env");
  process.env.NOTIGNORED_BIN = "";
  process.env.PATH = binaryOnPath(t);

  const report = await scan(["."], { cwd: dir });

  assert.deepEqual(
    report.ignores.map((d) => d.rules),
    [["F401"]],
  );
});

test("the bin option outranks the environment", unix, async (t) => {
  isolatedEnvironment(t);
  const dir = withFixture(t, "bin-wins");
  process.env.NOTIGNORED_BIN = notNotignored(t, "angry");

  const report = await scan(["."], { cwd: dir, bin: BIN });

  assert.deepEqual(
    report.ignores.map((d) => d.rules),
    [["F401"]],
  );
});

test("a bin that is not executable rejects as a spawn failure", async (t) => {
  const dir = scratch(t, "unspawnable");
  const notAProgram = file(dir, "notes.txt", "this is not a program\n");

  await assert.rejects(scan(["."], { cwd: dir, bin: notAProgram }), (error) => {
    assert.ok(error instanceof NotignoredSpawnError, `${error.name} is not a spawn error`);
    assert.match(error.message, /cannot run/);
    assert.ok(error.cause !== undefined, "the operating system's own error is carried");
    return true;
  });
});

test("an empty bin is refused before anything is spawned", async () => {
  await assert.rejects(scan(["."], { bin: "" }), NotignoredUsageError);
});

test(
  "a non-zero exit with nothing on stdout carries the exit code and stderr",
  unix,
  async (t) => {
    await assert.rejects(scan(["."], { bin: notNotignored(t, "angry") }), (error) => {
      assert.ok(error instanceof NotignoredExitError, `${error.name} is not an exit error`);
      assert.equal(error.exitCode, 3);
      assert.equal(error.signal, null);
      assert.match(error.stderr, /something went wrong/);
      return true;
    });
  },
);

test("output that is not JSON is a contract failure", unix, async (t) => {
  await assert.rejects(scan(["."], { bin: notNotignored(t, "text") }), (error) => {
    assert.ok(
      error instanceof NotignoredContractError,
      `${error.name} is not a contract error`,
    );
    assert.match(error.message, /did not return JSON/);
    return true;
  });
});

test("JSON that is not a report envelope is a contract failure", unix, async (t) => {
  await assert.rejects(scan(["."], { bin: notNotignored(t, "not-object") }), (error) => {
    assert.ok(
      error instanceof NotignoredContractError,
      `${error.name} is not a contract error`,
    );
    assert.match(error.message, /the report should be an object/);
    return true;
  });
});

test("a tool the contract does not have is refused, never skipped", unix, async (t) => {
  await assert.rejects(scan(["."], { bin: notNotignored(t, "bad-tool") }), (error) => {
    assert.ok(
      error instanceof NotignoredContractError,
      `${error.name} is not a contract error`,
    );
    assert.match(error.message, /ignores\[0\]\.tool should be one of/);
    assert.match(error.message, /"flake8"/);
    return true;
  });
});

test("a scope the contract does not have is refused too", unix, async (t) => {
  await assert.rejects(scan(["."], { bin: notNotignored(t, "bad-scope") }), (error) => {
    assert.ok(
      error instanceof NotignoredContractError,
      `${error.name} is not a contract error`,
    );
    assert.match(
      error.message,
      /ignores\[0\]\.scope should be one of line, next-line, file, block/,
    );
    return true;
  });
});

test("an envelope version that is not a number is refused", unix, async (t) => {
  await assert.rejects(scan(["."], { bin: notNotignored(t, "bad-version") }), (error) => {
    assert.ok(
      error instanceof NotignoredContractError,
      `${error.name} is not a contract error`,
    );
    assert.match(error.message, /the report.version should be a non-negative integer/);
    return true;
  });
});

test("a directive missing a field names the field", unix, async (t) => {
  await assert.rejects(scan(["."], { bin: notNotignored(t, "missing") }), (error) => {
    assert.match(error.message, /ignores\[0\]\.column should be a positive integer/);
    return true;
  });
});

/**
 * The same boundary the crate's deserializer holds: a newer envelope may carry
 * fields these types drop, and handing the caller a truncated report is worse
 * than refusing it.
 */
test("an envelope from a newer build is refused with the upgrade to make", unix, async (t) => {
  await assert.rejects(scan(["."], { bin: notNotignored(t, "future") }), (error) => {
    assert.ok(
      error instanceof NotignoredContractError,
      `${error.name} is not a contract error`,
    );
    assert.match(error.message, /version 2 is newer than this SDK understands \(1\)/);
    assert.match(error.message, /upgrade notignored-sdk/);
    return true;
  });
});

test("a tool name the contract does not have is refused before spawning", async () => {
  await assert.rejects(scan(["."], { tools: ["flake8"], bin: BIN }), (error) => {
    assert.ok(error instanceof NotignoredUsageError, `${error.name} is not a usage error`);
    assert.match(error.message, /unknown tool "flake8"/);
    assert.match(error.message, /ruff/);
    return true;
  });
});

test("tools must be an array", async () => {
  await assert.rejects(scan(["."], { tools: "ruff", bin: BIN }), NotignoredUsageError);
});

test("options must be an object", async () => {
  await assert.rejects(scan(["."], null), NotignoredUsageError);
});

test("cwd must be a non-empty string", async () => {
  await assert.rejects(scan(["."], { cwd: "", bin: BIN }), NotignoredUsageError);
});
