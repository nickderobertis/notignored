/**
 * What every journey in this suite needs: the real binary, scratch trees, and a
 * way to run a command and see everything it printed when it fails.
 *
 * Nothing here stands in for the subprocess. The SDK's whole job is to run
 * `notignored` and read what it returns, so a suite that faked either end would
 * prove only that its fake matched its expectations.
 */

import { spawnSync } from "node:child_process";
import {
  chmodSync,
  copyFileSync,
  existsSync,
  mkdtempSync,
  rmSync,
  symlinkSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

/** The project directory (`npm/notignored-sdk`). */
export const PROJECT = resolve(dirname(fileURLToPath(import.meta.url)), "..");

/** The repository root. */
export const REPO_ROOT = resolve(PROJECT, "..", "..");

const EXE = process.platform === "win32" ? ".exe" : "";

/**
 * Run a command to completion, returning its stdout — or throw with everything
 * it printed. A failure inside the harness must never read as a failure of the
 * behaviour under test.
 */
export function run(what, command, args, options = {}) {
  const result = spawnSync(command, args, {
    encoding: "utf8",
    ...options,
  });
  if (result.error) {
    throw new Error(`${what}: cannot run ${command}: ${result.error.message}`);
  }
  if (result.status !== 0) {
    throw new Error(
      `${what}: ${command} exited ${result.status}\n${result.stdout}\n${result.stderr}`,
    );
  }
  return result.stdout;
}

/**
 * The compiled `notignored` these journeys drive, built if it is not there yet.
 *
 * Debug over release: it is what the crate's own suite compiles, so a full gate
 * run has already paid for it. Building rather than skipping is deliberate — a
 * journey that quietly stopped running would report an unproven SDK as proven.
 */
export function binary() {
  for (const profile of ["debug", "release"]) {
    const candidate = join(REPO_ROOT, "target", profile, `notignored${EXE}`);
    if (existsSync(candidate)) return candidate;
  }
  const built = spawnSync("cargo", ["build", "--locked", "--bin", "notignored"], {
    cwd: REPO_ROOT,
    encoding: "utf8",
  });
  if (built.error || built.status !== 0) {
    throw new Error(
      "no notignored binary, and `cargo build --bin notignored` could not make one:\n" +
        `${built.error?.message ?? ""}${built.stderr ?? ""}\n` +
        "ACTION: run `just bootstrap`, then `cargo build --bin notignored`",
    );
  }
  return join(REPO_ROOT, "target", "debug", `notignored${EXE}`);
}

/**
 * A scratch directory removed when `t` finishes, whatever the outcome.
 *
 * Under the OS temp directory rather than the repository: several journeys
 * initialise a git repository, and one nested inside this one would take its
 * history and its .gitignore.
 */
export function scratch(t, label) {
  const dir = mkdtempSync(join(tmpdir(), `notignored-sdk-${label}-`));
  t.after(() => rmSync(dir, { recursive: true, force: true }));
  return dir;
}

/** Write `contents` to `dir/name`, returning the path. */
export function file(dir, name, contents) {
  const path = join(dir, name);
  writeFileSync(path, contents);
  return path;
}

/** Run git in `dir`. */
export function git(dir, ...args) {
  return run(`git ${args[0]}`, "git", args, { cwd: dir });
}

/**
 * A git repository with one commit, isolated from the ambient git config so a
 * developer's `commit.gpgsign` or hook path cannot decide whether this passes.
 */
export function gitRepo(t, label) {
  const dir = scratch(t, label);
  git(dir, "init", "--quiet", "--initial-branch=main");
  git(dir, "config", "user.email", "suite@example.invalid");
  git(dir, "config", "user.name", "notignored suite");
  git(dir, "config", "commit.gpgsign", "false");
  git(dir, "config", "core.hooksPath", join(dir, "no-such-hooks"));
  return dir;
}

/** Commit everything in `dir` with `message`. */
export function commit(dir, message) {
  git(dir, "add", "--all");
  git(dir, "commit", "--quiet", "--message", message);
}

/**
 * An executable copy of `test/fixtures/not-notignored.mjs` in `mode`.
 *
 * Copied rather than run in place, because the mode travels in the file name
 * and because the committed fixture then needs no executable bit of its own.
 */
export function notNotignored(t, mode) {
  const dir = scratch(t, `not-notignored-${mode}`);
  const path = join(dir, `notignored-${mode}`);
  copyFileSync(join(PROJECT, "test", "fixtures", "not-notignored.mjs"), path);
  chmodSync(path, 0o755);
  return path;
}

/**
 * A directory holding nothing but a `notignored` that is the real binary, for
 * the journeys that ask what `PATH` resolution finds.
 */
export function binaryOnPath(t) {
  const dir = scratch(t, "path");
  symlinkSync(binary(), join(dir, `notignored${EXE}`));
  return dir;
}
