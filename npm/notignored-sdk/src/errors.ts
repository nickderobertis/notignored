/**
 * Everything `scan` can reject with, as one hierarchy.
 *
 * A caller catches {@link NotignoredError} to mean "the scan did not happen";
 * the subclasses say which boundary refused, because the fix differs at each of
 * them. Nothing here is thrown for a *finding* — a report full of suppressions,
 * or one carrying `errors` for files the binary could not read, resolves.
 */

/** Base class for every failure this SDK raises. */
export class NotignoredError extends Error {
  constructor(message: string, options?: { cause?: unknown }) {
    super(message, options);
    this.name = "NotignoredError";
  }
}

/**
 * The arguments handed to `scan` do not describe a run.
 *
 * Raised before anything is spawned, so a rejected call has no side effects.
 */
export class NotignoredUsageError extends NotignoredError {
  constructor(message: string) {
    super(message);
    this.name = "NotignoredUsageError";
  }
}

/**
 * No `notignored` binary could be resolved.
 *
 * The message names every way to install one, because a caller who hits this
 * has nothing else to go on: there is no exit code and no stderr to read.
 */
export class NotignoredBinaryNotFoundError extends NotignoredError {
  constructor(message: string) {
    super(message);
    this.name = "NotignoredBinaryNotFoundError";
  }
}

/**
 * A binary was resolved, and the operating system refused to run it.
 *
 * Distinct from {@link NotignoredBinaryNotFoundError}: something *is* there —
 * it is not executable, or is not a program — so the fix is about that file
 * rather than about installing anything.
 */
export class NotignoredSpawnError extends NotignoredError {
  constructor(message: string, options: { cause: unknown }) {
    super(message, options);
    this.name = "NotignoredSpawnError";
  }
}

/**
 * The binary ran and failed.
 *
 * `stderr` is carried verbatim: `notignored` writes the reason and a concrete
 * hint there, and re-wording either would lose the fix it names.
 */
export class NotignoredExitError extends NotignoredError {
  /** The process exit code, or `null` when a signal ended it. */
  readonly exitCode: number | null;
  /** The signal that ended the process, or `null` when it exited normally. */
  readonly signal: string | null;
  /** Everything the process wrote to stderr. */
  readonly stderr: string;

  constructor(
    message: string,
    details: { exitCode: number | null; signal: string | null; stderr: string },
  ) {
    super(message);
    this.name = "NotignoredExitError";
    this.exitCode = details.exitCode;
    this.signal = details.signal;
    this.stderr = details.stderr;
  }
}

/**
 * The binary exited cleanly and what it wrote is not a report.
 *
 * The versioned record contract is what this SDK promises its callers, so an
 * envelope that does not hold it is refused here rather than handed on as a
 * half-typed object. Reaching this usually means the resolved binary is not
 * `notignored`, or is a build newer than this SDK.
 */
export class NotignoredContractError extends NotignoredError {
  constructor(message: string, options?: { cause?: unknown }) {
    super(message, options);
    this.name = "NotignoredContractError";
  }
}
