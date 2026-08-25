# notignored-sdk

Typed TypeScript SDK for [notignored](https://github.com/nickderobertis/notignored):
every lint and type-check suppression comment in a source tree — `# noqa`,
`// eslint-disable-next-line`, `#[allow(…)]`, and the rest — as records you can
query, from Node.

It drives the real `notignored` binary and mirrors the CLI, so a vitest suite, a
GitHub Actions step, and `notignored` on your terminal all report the same thing.

## Install

```bash
npm install notignored-sdk notignored-cli
```

`notignored-cli` carries the prebuilt binary; the SDK runs whatever it finds.
Already have `notignored` on `PATH` (from `pip install notignored-cli`,
`cargo install`, or the install script)? Then the SDK is all you need.

## Use

```ts
import { scan } from "notignored-sdk";

const report = await scan(["src"]);
for (const directive of report.ignores) {
  const rules = directive.rules.join(", ") || "every rule";
  console.log(
    `${directive.path}:${directive.line} silences ${rules} (${directive.scope})` +
      ` — ${directive.reason ?? "no reason given"}`,
  );
}
```

The review case — only the suppressions this branch added:

```ts
const added = await scan(["."], { diff: true, diffBase: "origin/main" });
if (added.ignores.some((directive) => directive.reason === null)) {
  throw new Error("this change adds a suppression with no stated reason");
}
```

Each of those records carries a `change`: `"added"` when the branch wrote the
suppression (or altered which rules, or how far, an existing one silences), and
`"justification-edited"` when it was already there and what moved is its stated
reason. It is `null` on any scan that is not a diff, which has no base to have
changed anything against.

### `scan(paths?, options?)`

| | |
| --- | --- |
| `paths` | Files and directories to scan; directories are walked recursively, honouring `.gitignore`. Defaults to the working directory. |
| `options.diff` | Report only the suppressions this change added (`--diff`). |
| `options.diffBase` | The git revision or range to compare against (`--diff-base`). Requires `diff: true`. |
| `options.tools` | Report only these tools (`--tool`): `eslint`, `biome`, `ruff`, `typescript`, `mypy`, `pyright`, `ty`, `rust`, `shellcheck`, `llmlint`. |
| `options.cwd` | Where to run. Report paths are relative to it. |

Those four are the whole options bag; there is no fifth.

It resolves to a `Report` — the same versioned JSON contract `notignored
--format json` prints, field for field:

```ts
interface Report {
  version: number;
  ignores: IgnoreDirective[];
  errors: ReportError[];
}
```

Only a run that completed resolves. `notignored` exits non-zero when it could
not read a file — even though it still prints the envelope, with that file in
`errors` — so `scan` rejects with `NotignoredExitError` rather than hand back a
report of a scan that skipped part of the tree. Read `exitCode` and `stderr` to
decide what to do about it.

### Finding the binary

In order: the `NOTIGNORED_BIN` environment variable, the `notignored-cli`
package installed beside this one, then `PATH`. With none of them, `scan`
rejects with `NotignoredBinaryNotFoundError` and the ways to install one.

There is no call-site option for this on purpose — one mechanism, settable by a
CI step or a test harness without touching any call site.

### Errors

Every rejection is a `NotignoredError`:

| | |
| --- | --- |
| `NotignoredUsageError` | The call does not describe a run (`diffBase` without `diff`, an unknown tool). Nothing was spawned. |
| `NotignoredBinaryNotFoundError` | No `notignored` could be resolved. |
| `NotignoredSpawnError` | One was resolved and could not be started; `cause` is the operating system's error. |
| `NotignoredExitError` | It exited non-zero, for any reason; `exitCode`, `signal`, and verbatim `stderr` say which. |
| `NotignoredContractError` | A clean run's output is not a report this SDK can read — a missing or mistyped field, an unknown `tool`, `scope` or `change`, or an envelope from a newer build. A field this SDK has never heard of is carried past, not refused. |

## Working on it

From the repository root:

```bash
just bootstrap                          # provisions every project
just nx run notignored-sdk-npm:check    # this project's gate alone
just check                              # the whole repo's gate
```

## License

MIT
