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
//   bad-tool     write a report naming a tool the contract does not have
//   bad-scope    write a report naming a scope the contract does not have
//   bad-version  write an envelope whose version is not a number
//   missing      write a report whose directive has lost a field
//   future       write an envelope from a version this SDK cannot read
//   angry        write nothing on stdout, a reason on stderr, exit 3

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

function directive(changes) {
  const report = structuredClone(REPORT);
  Object.assign(report.ignores[0], changes);
  return report;
}

function without(field) {
  const report = structuredClone(REPORT);
  delete report.ignores[0][field];
  return report;
}

const MODES = {
  "not-object": () => process.stdout.write("[]"),
  "bad-tool": () => process.stdout.write(JSON.stringify(directive({ tool: "flake8" }))),
  "bad-scope": () => process.stdout.write(JSON.stringify(directive({ scope: "paragraph" }))),
  "bad-version": () => process.stdout.write(JSON.stringify({ ...REPORT, version: "1" })),
  missing: () => process.stdout.write(JSON.stringify(without("column"))),
  future: () => process.stdout.write(JSON.stringify({ ...REPORT, version: 2 })),
  text: () => process.stdout.write("notignored 9.9.9\n"),
  angry: () => {
    process.stderr.write("notignored: something went wrong\n");
    process.exit(3);
  },
};

const mode = Object.keys(MODES).find((name) => process.argv[1].endsWith(`-${name}`));
if (mode === undefined) {
  process.stderr.write(
    `not-notignored: ${process.argv[1]} names no mode (one of ${Object.keys(MODES).join(", ")})\n`,
  );
  process.exit(64);
}
MODES[mode]();
