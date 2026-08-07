/**
 * Which `notignored` a scan runs, and what to say when there is none.
 *
 * Four sources, most explicit first: the `bin` option, the `NOTIGNORED_BIN`
 * environment variable, the `notignored-cli` npm launcher installed beside this
 * package, then `PATH`. The launcher outranks `PATH` for the same reason npm
 * prefers a project's own tooling to whatever is installed globally: a project
 * that pinned a version meant that version.
 */

import { accessSync, constants, statSync } from "node:fs";
import { createRequire } from "node:module";
import { delimiter, join } from "node:path";

import { NotignoredBinaryNotFoundError } from "./errors.js";

/** The environment variable that overrides binary resolution. */
export const BINARY_ENV_VAR = "NOTIGNORED_BIN";

/** How the command is spelled on `PATH`. */
const COMMAND = process.platform === "win32" ? "notignored.exe" : "notignored";

/**
 * Every way to get a binary, in the order a reader should try them.
 *
 * This is the whole of what {@link NotignoredBinaryNotFoundError} can offer, so
 * it names each install path rather than only the one this package ships with.
 */
const INSTALL_GUIDANCE =
  "no notignored binary found. Install one with `npm install notignored-cli` " +
  "(or `-g`), `pip install notignored-cli`, or " +
  "`cargo install --git https://github.com/nickderobertis/notignored --locked`; " +
  `or point the ${BINARY_ENV_VAR} environment variable (or scan's \`bin\` option) at an existing one.`;

/** A resolved command and the arguments that must precede the scan's own. */
export interface ResolvedBinary {
  /** The program to spawn. */
  command: string;
  /** Arguments the program needs before the scan's, if any. */
  prefix: string[];
}

/** Whether `candidate` is a file this process may execute. */
function executable(candidate: string): boolean {
  try {
    if (!statSync(candidate).isFile()) return false;
    accessSync(candidate, constants.X_OK);
    return true;
  } catch {
    return false;
  }
}

/**
 * The npm launcher installed alongside this package, if it is there.
 *
 * Resolved as a *module* rather than looked for on `PATH`: a dependency
 * installed into a project's `node_modules` puts its command on `PATH` only
 * inside an npm script, and an SDK is usually called from neither.
 */
function launcher(): string | undefined {
  try {
    return createRequire(import.meta.url).resolve("notignored-cli/bin/notignored.js");
  } catch {
    return undefined;
  }
}

/**
 * The first executable `notignored` on `PATH`.
 *
 * Searched here rather than left to `spawn` so that "nothing is installed" is a
 * typed error with install guidance instead of a bare `ENOENT`. On Windows only
 * a real `.exe` counts: Node refuses to spawn a `.cmd` without a shell, and
 * putting user-supplied paths through one would be an injection. A global npm
 * install is reached through the launcher above on that platform.
 */
function onPath(): string | undefined {
  for (const entry of (process.env.PATH ?? "").split(delimiter)) {
    if (entry === "") continue;
    const candidate = join(entry, COMMAND);
    if (executable(candidate)) return candidate;
  }
  return undefined;
}

/**
 * Decide which binary to run.
 *
 * @throws {NotignoredBinaryNotFoundError} when no source yields one.
 */
export function resolveBinary(bin?: string | undefined): ResolvedBinary {
  if (bin !== undefined) return { command: bin, prefix: [] };

  const override = process.env[BINARY_ENV_VAR];
  if (override !== undefined && override !== "") {
    return { command: override, prefix: [] };
  }

  const shim = launcher();
  if (shim !== undefined) return { command: process.execPath, prefix: [shim] };

  const found = onPath();
  if (found !== undefined) return { command: found, prefix: [] };

  throw new NotignoredBinaryNotFoundError(INSTALL_GUIDANCE);
}
