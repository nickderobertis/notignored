/**
 * Typed TypeScript SDK for [notignored](https://github.com/nickderobertis/notignored).
 *
 * One entry point — {@link scan} — over the real `notignored` binary, mirroring
 * the CLI: a tree scan, a `--diff` scan, and the `--tool` filter. It always asks
 * for `--format json` and hands back the versioned record contract, so a caller
 * gets the same records the GitHub Action and the Python SDK see.
 *
 * ```ts
 * import { scan } from "notignored-sdk";
 *
 * const report = await scan(["src"], { tools: ["ruff"] });
 * for (const directive of report.ignores) {
 *   console.log(`${directive.path}:${directive.line} ${directive.rules.join(",")}`);
 * }
 * ```
 */

import { spawn } from "node:child_process";

import { resolveBinary } from "./binary.js";
import { isTool, parseReport, toolNames, type Report, type Tool } from "./contract.js";
import { NotignoredExitError, NotignoredSpawnError, NotignoredUsageError } from "./errors.js";

export type {
  IgnoreDirective,
  Report,
  ReportError,
  Scope,
  Suppressed,
  Tool,
} from "./contract.js";
export {
  NotignoredBinaryNotFoundError,
  NotignoredContractError,
  NotignoredError,
  NotignoredExitError,
  NotignoredSpawnError,
  NotignoredUsageError,
} from "./errors.js";

/** How one {@link scan} runs. */
export interface ScanOptions {
  /**
   * Report only the suppressions this change added, the way `--diff` does:
   * git names the changed files, and only directives on added lines survive.
   */
  diff?: boolean | undefined;
  /**
   * The git revision or range `diff` is taken from. Requires `diff`, exactly as
   * the CLI's `--diff-base` requires `--diff`.
   */
  diffBase?: string | undefined;
  /** Report only these tools' directives; omit for all of them. */
  tools?: readonly Tool[] | undefined;
  /** Where to run. Report paths are relative to it. Defaults to this process's. */
  cwd?: string | undefined;
  /**
   * The binary to run, overriding every other source. Otherwise `NOTIGNORED_BIN`,
   * then the `notignored-cli` npm launcher, then `PATH`.
   */
  bin?: string | undefined;
}

/** Reject anything that is not a usable, non-empty string. */
function requireText(value: unknown, what: string): string {
  if (typeof value !== "string" || value === "") {
    throw new NotignoredUsageError(`${what} must be a non-empty string`);
  }
  return value;
}

/**
 * The command line one call implies.
 *
 * Every caller-supplied path goes after `--`. A path is data, and without the
 * separator one that begins with a dash would be read as a flag — turning
 * `scan(["--fail-if-found"])` into a different command than the one asked for.
 */
function argumentsFor(paths: readonly string[], options: ScanOptions): string[] {
  const args = ["--format", "json"];

  if (options.tools !== undefined) {
    if (!Array.isArray(options.tools)) {
      throw new NotignoredUsageError("tools must be an array of tool names");
    }
    for (const tool of options.tools) {
      if (!isTool(tool)) {
        throw new NotignoredUsageError(
          `unknown tool ${JSON.stringify(tool)} (known tools: ${toolNames()})`,
        );
      }
      args.push("--tool", tool);
    }
  }

  if (options.diffBase !== undefined && options.diff !== true) {
    throw new NotignoredUsageError(
      "diffBase requires diff: true, the same way --diff-base requires --diff",
    );
  }
  if (options.diff === true) args.push("--diff");
  if (options.diffBase !== undefined) {
    args.push("--diff-base", requireText(options.diffBase, "diffBase"));
  }

  args.push("--");
  for (const path of paths) args.push(requireText(path, "every path"));
  return args;
}

/**
 * Scan a source tree and return every suppression comment in it.
 *
 * @param paths Files and/or directories to scan. Directories are walked
 *   recursively, honouring `.gitignore`. Defaults to the working directory.
 * @param options See {@link ScanOptions}.
 *
 * A report the binary produced always resolves, even when the binary exited
 * non-zero because it could not read one of the files: that file is in
 * `report.errors`, which is where the contract puts it, and losing the rest of
 * the scan to it would hide the suppressions that *were* found. A run that
 * produced no report at all rejects with {@link NotignoredExitError}.
 *
 * @throws {NotignoredUsageError} when the arguments do not describe a run.
 * @throws {NotignoredBinaryNotFoundError} when no binary could be resolved.
 * @throws {NotignoredSpawnError} when the resolved binary could not be started.
 * @throws {NotignoredExitError} when it failed without producing a report.
 * @throws {NotignoredContractError} when what it produced is not one.
 */
export async function scan(
  paths: readonly string[] = [],
  options: ScanOptions = {},
): Promise<Report> {
  if (!Array.isArray(paths)) {
    throw new NotignoredUsageError("paths must be an array of strings");
  }
  if (typeof options !== "object" || options === null) {
    throw new NotignoredUsageError("options must be an object");
  }
  // The arguments are checked before the environment is consulted: whether a
  // call describes a run is a property of the call alone, and a caller with
  // nothing installed should still be told which of the two is wrong first.
  const cwd = options.cwd === undefined ? undefined : requireText(options.cwd, "cwd");
  const args = argumentsFor(paths, options);
  const binary = resolveBinary(
    options.bin === undefined ? undefined : requireText(options.bin, "bin"),
  );

  return await new Promise<Report>((resolve, reject) => {
    const child = spawn(binary.command, [...binary.prefix, ...args], {
      ...(cwd === undefined ? {} : { cwd }),
      windowsHide: true,
    });
    let stdout = "";
    let stderr = "";
    let failed = false;
    child.stdout.setEncoding("utf8").on("data", (chunk: string) => {
      stdout += chunk;
    });
    child.stderr.setEncoding("utf8").on("data", (chunk: string) => {
      stderr += chunk;
    });
    child.on("error", (cause) => {
      failed = true;
      reject(
        new NotignoredSpawnError(`cannot run ${binary.command}: ${cause.message}`, { cause }),
      );
    });
    child.on("close", (code, signal) => {
      // A spawn failure emits both `error` and `close`; the first one settled
      // the promise and this is the tail of it.
      if (failed) return;
      if (code === 0) {
        try {
          resolve(parseReport(stdout));
        } catch (error) {
          reject(error as Error);
        }
        return;
      }
      // A non-zero exit that still printed the envelope is the documented
      // "some file could not be read" case, and the report carries the reason.
      // Anything else — a bad path, a diff that could not be taken — printed
      // its explanation on stderr instead, so that is what the caller gets.
      try {
        resolve(parseReport(stdout));
      } catch {
        reject(
          new NotignoredExitError(
            `notignored exited ${signal === null ? code : signal}: ${stderr.trim()}`,
            { exitCode: code, signal, stderr },
          ),
        );
      }
    });
  });
}
