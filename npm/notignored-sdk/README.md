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

### `scan(paths?, options?)`

| | |
| --- | --- |
| `paths` | Files and directories to scan; directories are walked recursively, honouring `.gitignore`. Defaults to the working directory. |
| `options.diff` | Report only the suppressions this change added (`--diff`). |
| `options.diffBase` | The git revision or range to compare against (`--diff-base`). Requires `diff: true`. |
| `options.tools` | Report only these tools (`--tool`): `eslint`, `biome`, `ruff`, `typescript`, `mypy`, `pyright`, `ty`, `rust`, `shellcheck`, `llmlint`. |
| `options.cwd` | Where to run. Report paths are relative to it. |
| `options.bin` | The binary to run, overriding every other source. |

It resolves to a `Report` — the same versioned JSON contract `notignored
--format json` prints, field for field:

```ts
interface Report {
  version: number;
  ignores: IgnoreDirective[];
  errors: ReportError[];
}
```

A file the binary could not read is a `ReportError` in `report.errors`, not an
exception: the rest of the scan is still what a reviewer needs.

### Finding the binary

In order: `options.bin`, the `NOTIGNORED_BIN` environment variable, the
`notignored-cli` package installed beside this one, then `PATH`. With none of
them, `scan` rejects with `NotignoredBinaryNotFoundError` and the ways to
install one.

### Errors

Every rejection is a `NotignoredError`:

| | |
| --- | --- |
| `NotignoredUsageError` | The call does not describe a run (`diffBase` without `diff`, an unknown tool). Nothing was spawned. |
| `NotignoredBinaryNotFoundError` | No `notignored` could be resolved. |
| `NotignoredSpawnError` | One was resolved and could not be started; `cause` is the operating system's error. |
| `NotignoredExitError` | It ran and failed; `exitCode`, `signal`, and `stderr` say why. |
| `NotignoredContractError` | What it returned is not a report this SDK can read — including an envelope from a newer build. |

## Working on it

From the repository root:

```bash
just bootstrap                          # provisions every project
just nx run notignored-sdk-npm:check    # this project's gate alone
just check                              # the whole repo's gate
```

## License

MIT
