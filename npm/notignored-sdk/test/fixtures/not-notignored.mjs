#!/usr/bin/env node
// A program that is not `notignored`, for the branches only a wrong binary can
// reach.
//
// Not a stand-in for the real one: every journey that asks what a scan *finds*
// drives the compiled binary. These are the branches where the resolved command
// turns out to be something else — a name collision on PATH, a stale override,
// a build newer than this SDK — and the real binary cannot produce them.
//
// The suite copies this file under a name ending in the mode it wants, so the
// mode travels with the path the SDK was pointed at rather than through an
// environment variable a neighbouring test could still be holding:
//
//   text         write something that is not JSON, exit 0
//   not-object   write a JSON array, exit 0
//   bad-tool     a tool the contract does not have
//   bad-scope    a scope the contract does not have
//   bad-version  an envelope whose version is not a number
//   future       an envelope from a version this SDK cannot read
//   angry        nothing on stdout, a reason on stderr, exit 3
//
// One pair per object boundary — a field the contract does not define, and a
// field it requires:
//
//   extra-report      missing-errors
//   extra-directive   missing-column
//   extra-suppressed  missing-start-line
//   extra-error       missing-message

const REPORT = {
  version: 1,
  ignores: [
    {
      tool: "ruff",
      scope: "line",
      rules: ["F401"],
      reason: null,
      path: "app.py",
      line: 1,
      end_line: 1,
      column: 12,
      raw: "# noqa: F401",
      suppressed: { start_line: 1, end_line: 1 },
    },
  ],
  errors: [],
};

/** The envelope with the error entry the real binary emits for a bad file. */
const WITH_ERROR = {
  version: 1,
  ignores: [],
  errors: [{ path: "locked.py", message: "Permission denied (os error 13)" }],
};

/** A copy of `base` that `edit` has changed, ready to print. */
function altered(base, edit) {
  const copy = structuredClone(base);
  edit(copy);
  return copy;
}

const MODES = {
  "not-object": () => [],
  "bad-tool": () =>
    altered(REPORT, (report) => {
      report.ignores[0].tool = "flake8";
    }),
  "bad-scope": () =>
    altered(REPORT, (report) => {
      report.ignores[0].scope = "paragraph";
    }),
  "bad-version": () => ({ ...REPORT, version: "1" }),
  future: () => ({ ...REPORT, version: 2 }),

  "extra-report": () =>
    altered(REPORT, (report) => {
      report.superseded_by = "nothing";
    }),
  "extra-directive": () =>
    altered(REPORT, (report) => {
      report.ignores[0].severity = "high";
    }),
  "extra-suppressed": () =>
    altered(REPORT, (report) => {
      report.ignores[0].suppressed.end_column = 40;
    }),
  "extra-error": () =>
    altered(WITH_ERROR, (report) => {
      report.errors[0].kind = "io";
    }),

  "missing-errors": () =>
    altered(REPORT, (report) => {
      delete report.errors;
    }),
  "missing-column": () =>
    altered(REPORT, (report) => {
      delete report.ignores[0].column;
    }),
  "missing-start-line": () =>
    altered(REPORT, (report) => {
      delete report.ignores[0].suppressed.start_line;
    }),
  "missing-message": () =>
    altered(WITH_ERROR, (report) => {
      delete report.errors[0].message;
    }),
};

const mode = Object.keys(MODES).find((name) => process.argv[1].endsWith(`-${name}`));

if (process.argv[1].endsWith("-angry")) {
  process.stderr.write("notignored: something went wrong\n");
  process.exit(3);
}
if (process.argv[1].endsWith("-text")) {
  process.stdout.write("notignored 9.9.9\n");
  process.exit(0);
}
if (mode === undefined) {
  process.stderr.write(
    `not-notignored: ${process.argv[1]} names no mode (one of ${Object.keys(MODES).join(", ")}, text, angry)\n`,
  );
  process.exit(64);
}
process.stdout.write(JSON.stringify(MODES[mode]()));
