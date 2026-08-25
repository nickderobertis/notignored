/**
 * The versioned record contract, mirrored field for field, and the strict
 * reader that turns one process's stdout into it.
 *
 * The names here are the JSON's names — `start_line`, not `startLine`. A
 * camelCase mirror would be a second spelling of a published contract, and the
 * first field added upstream would land in only one of them.
 *
 * Validation is strict about what the contract *specifies*: a missing or
 * mistyped field, an unknown tool or scope, an envelope from a newer build.
 * `notignored` promises these records, so an envelope that does not hold the
 * shape is a broken promise somewhere upstream — a resolved binary that is not
 * `notignored`, or one newer than this SDK — and a silently-skipped directive
 * is the one outcome a suppression reporter must never produce.
 *
 * It is **tolerant of keys it has never heard of**, because the record
 * contract's own rule is that new fields are optional and additive. This reader
 * used to refuse them, on the reasoning that a field it dropped might change
 * what a record means; the price turned out to be higher than the protection —
 * every additive field the crate adds within version 1 would break every
 * consumer holding an older SDK, over something none of them reads. The version
 * check is where an envelope that really has changed meaning is caught.
 */

import { NotignoredContractError } from "./errors.js";

/**
 * Envelope version this SDK understands.
 *
 * Kept in step with the crate's `REPORT_VERSION`. A report claiming a higher
 * one may carry fields these types drop, so it is refused rather than truncated
 * — the same boundary the Rust deserializer holds.
 */
const REPORT_VERSION = 1;

/** Every tool whose suppression comments `notignored` understands. */
const TOOLS = [
  "eslint",
  "biome",
  "ruff",
  "typescript",
  "mypy",
  "pyright",
  "ty",
  "rust",
  "shellcheck",
  "llmlint",
] as const;

/** How far a directive's suppression reaches. */
const SCOPES = ["line", "next-line", "file", "block"] as const;

/** What a `--diff` scan's change did to a suppression. */
const CHANGES = ["added", "justification-edited"] as const;

/** A lint or type-check tool whose suppression comments are reported. */
export type Tool = (typeof TOOLS)[number];

/** How far a directive's suppression reaches. */
export type Scope = (typeof SCOPES)[number];

/**
 * What a `--diff` scan's change did to a suppression.
 *
 * `justification-edited` says the *justification* moved and nothing else did; a
 * directive whose rules or scope the change altered is `added`, because it now
 * silences something its base version did not.
 */
export type Change = (typeof CHANGES)[number];

/** The best-effort range of source lines a directive silences. */
export interface Suppressed {
  /** First 1-based line the directive silences. */
  start_line: number;
  /** Last 1-based line, or `null` when the range runs to end-of-file. */
  end_line: number | null;
}

/** One parsed suppression comment. */
export interface IgnoreDirective {
  /** The tool whose rules are being silenced. */
  tool: Tool;
  /** How far the suppression reaches. */
  scope: Scope;
  /** Rule names/codes exactly as written; empty means every rule. */
  rules: string[];
  /** The stated justification, or `null` when none was given. */
  reason: string | null;
  /** Path to the file, relative to the scan's working directory. */
  path: string;
  /** 1-based line the directive starts on. */
  line: number;
  /** 1-based line the directive ends on. */
  end_line: number;
  /** 1-based column the directive starts at. */
  column: number;
  /** The directive exactly as it appears in the source. */
  raw: string;
  /** The range of lines this directive silences. */
  suppressed: Suppressed;
  /**
   * Whether the change introduced this suppression or rewrote the justification
   * of one that already existed, on a `--diff` scan.
   *
   * `null` on any scan that is not a `--diff` one: a tree scan has no base, so
   * there is nothing to have been added or edited against.
   */
  change: Change | null;
}

/** A file that could not be read, or a directive that could not be parsed. */
export interface ReportError {
  /** Path the problem was found at. */
  path: string;
  /** What went wrong, in one line. */
  message: string;
}

/** The report envelope: everything one scan produced. */
export interface Report {
  /** Envelope version. */
  version: number;
  /** Every directive found, ordered by path, then line, then column. */
  ignores: IgnoreDirective[];
  /** Files that could not be read and directives that could not be parsed. */
  errors: ReportError[];
}

/** Whether a value is one of the tools in the contract. */
export function isTool(value: unknown): value is Tool {
  return TOOLS.some((tool) => tool === value);
}

/** Every tool name, for the message a rejected `tools` option gets. */
export function toolNames(): string {
  return TOOLS.join(", ");
}

function reject(at: string, wanted: string, got: unknown): never {
  throw new NotignoredContractError(
    `notignored returned a report this SDK cannot read: ${at} should be ${wanted}, got ${describe(got)}`,
  );
}

/** A value as it reads in a diagnostic — short, and never the whole payload. */
function describe(value: unknown): string {
  if (value === null) return "null";
  if (Array.isArray(value)) return "an array";
  if (typeof value === "object") return "an object";
  const rendered = JSON.stringify(value) ?? String(value);
  return rendered.length > 40 ? `${rendered.slice(0, 40)}…` : rendered;
}

function record(value: unknown, at: string): Record<string, unknown> {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    reject(at, "an object", value);
  }
  return value as Record<string, unknown>;
}

function text(source: Record<string, unknown>, key: string, at: string): string {
  const value = source[key];
  if (typeof value !== "string") reject(`${at}.${key}`, "a string", value);
  return value;
}

function optionalText(source: Record<string, unknown>, key: string, at: string): string | null {
  const value = source[key];
  if (value === null) return null;
  if (typeof value !== "string") reject(`${at}.${key}`, "a string or null", value);
  return value;
}

/** A 1-based coordinate: the extractor's cursor can never emit zero. */
function coordinate(source: Record<string, unknown>, key: string, at: string): number {
  const value = source[key];
  if (typeof value !== "number" || !Number.isInteger(value) || value < 1) {
    reject(`${at}.${key}`, "a positive integer", value);
  }
  return value;
}

function optionalCoordinate(
  source: Record<string, unknown>,
  key: string,
  at: string,
): number | null {
  if (source[key] === null) return null;
  return coordinate(source, key, at);
}

function list(source: Record<string, unknown>, key: string, at: string): unknown[] {
  const value = source[key];
  if (!Array.isArray(value)) reject(`${at}.${key}`, "an array", value);
  return value;
}

function suppressed(value: unknown, at: string): Suppressed {
  const node = record(value, at);
  return {
    start_line: coordinate(node, "start_line", at),
    end_line: optionalCoordinate(node, "end_line", at),
  };
}

function directive(value: unknown, at: string): IgnoreDirective {
  const node = record(value, at);
  const tool = node.tool;
  if (!isTool(tool)) reject(`${at}.tool`, `one of ${toolNames()}`, tool);
  const scope = node.scope;
  if (!SCOPES.some((known) => known === scope)) {
    reject(`${at}.scope`, `one of ${SCOPES.join(", ")}`, scope);
  }
  return {
    tool,
    scope: scope as Scope,
    rules: list(node, "rules", at).map((rule, index) => {
      if (typeof rule !== "string") reject(`${at}.rules[${index}]`, "a string", rule);
      return rule;
    }),
    reason: optionalText(node, "reason", at),
    path: text(node, "path", at),
    line: coordinate(node, "line", at),
    end_line: coordinate(node, "end_line", at),
    column: coordinate(node, "column", at),
    raw: text(node, "raw", at),
    suppressed: suppressed(node.suppressed, `${at}.suppressed`),
    change: change(node, at),
  };
}

/**
 * The `change` a `--diff` scan wrote, or `null`.
 *
 * Absent is not a third value — it says the scan had no base to classify
 * against — so it reads as `null` here, exactly the way an unstated `reason`
 * does. A value the contract does not define is refused, for the same reason an
 * unknown tool is: a word this SDK cannot read is one it must not guess at.
 */
function change(source: Record<string, unknown>, at: string): Change | null {
  const value = source.change;
  if (value === undefined || value === null) return null;
  if (!CHANGES.some((known) => known === value)) {
    reject(`${at}.change`, `one of ${CHANGES.join(", ")}`, value);
  }
  return value as Change;
}

function reportError(value: unknown, at: string): ReportError {
  const node = record(value, at);
  return { path: text(node, "path", at), message: text(node, "message", at) };
}

/**
 * Read one `--format json` run's stdout as a {@link Report}.
 *
 * Throws {@link NotignoredContractError} for anything that is not one, which
 * includes an envelope from a build newer than this SDK.
 */
export function parseReport(stdout: string): Report {
  let payload: unknown;
  try {
    payload = JSON.parse(stdout) as unknown;
  } catch (cause) {
    throw new NotignoredContractError(
      `notignored did not return JSON: ${cause instanceof Error ? cause.message : String(cause)}`,
      { cause },
    );
  }
  const node = record(payload, "the report");
  const version = node.version;
  if (typeof version !== "number" || !Number.isInteger(version) || version < 0) {
    reject("the report.version", "a non-negative integer", version);
  }
  // The version is read before the envelope's own fields, so a build that
  // *said* it was newer gets the message that names the upgrade rather than a
  // complaint about whichever field it added.
  if (version > REPORT_VERSION) {
    throw new NotignoredContractError(
      `report version ${version} is newer than this SDK understands (${REPORT_VERSION}); upgrade notignored-sdk`,
    );
  }
  return {
    version,
    ignores: list(node, "ignores", "the report").map((entry, index) =>
      directive(entry, `the report.ignores[${index}]`),
    ),
    errors: list(node, "errors", "the report").map((entry, index) =>
      reportError(entry, `the report.errors[${index}]`),
    ),
  };
}
