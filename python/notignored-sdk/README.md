# notignored-sdk

Typed Python access to [`notignored`](https://github.com/nickderobertis/notignored):
every lint and type-check suppression comment in a source tree — `# noqa`,
`// eslint-disable-next-line`, `#[allow(...)]`, `# type: ignore`, and the rest —
as frozen records with the tool, the rules, and the stated reason.

The distribution is `notignored-sdk`, the import is `notignored_sdk`, and every
release depends on the exact `notignored-cli` it was built with, so
`pip install` brings a binary that speaks the same report contract.

```console
pip install notignored-sdk
```

```python
from notignored_sdk import scan

report = scan(["src"])
for directive in report.ignores:
    print(
        f"{directive.path}:{directive.line} {directive.tool} {directive.rules} — {directive.reason}"
    )
```

## The guard: prove nobody silenced a linter to make the gate pass

The case this exists for. A green `just check` means nothing if the way it went
green was a new `# noqa`, so pin the suppressions a branch is allowed to add and
fail when it adds one that is not justified:

```python
from notignored_sdk import scan


def test_this_branch_added_no_unjustified_suppression() -> None:
    """Every suppression this change adds has to say why it is there."""
    added = scan(diff=True, diff_base="origin/main")

    unexplained = [d for d in added.ignores if not d.reason]
    assert not unexplained, "\n".join(
        f"{d.path}:{d.line} adds `{d.raw}` with no stated reason" for d in unexplained
    )
```

`diff=True` reports only the suppressions on lines the change added, using the
same merge-base semantics as `notignored --diff` — so a suppression that was
already on `main` is never yours.

## The surface

One entry point, in two forms. `ascan` takes the identical arguments and returns
the identical report:

```python
scan(paths=(), *, diff=False, diff_base=None, tools=None, cwd=None) -> Report
await ascan(...)  # the same call, on an event loop
```

| Argument | What it does |
| --- | --- |
| `paths` | Files and/or directories. Directories are walked recursively, honouring `.gitignore`. Empty scans the working directory. |
| `diff` | Report only the suppressions this change added. |
| `diff_base` | The git revision `diff` compares against. Without `diff=True` it is a `ValueError`, exactly as the CLI rejects it. |
| `tools` | Report only these tools (`Tool` members or their names); `None` reports all of them. |
| `cwd` | Directory to run in. Report paths are relative to it. |

### Which binary runs

Resolution is, in order: **`NOTIGNORED_BIN`**, then the `notignored` on `PATH`.

`NOTIGNORED_BIN` *is* the explicit override — the entry points deliberately take
no binary argument, so their signatures carry the CLI's flags and nothing else.
`pip install notignored-sdk` puts the pinned binary on `PATH` for you, so most
callers never set it; point it at a specific build when you need one, as this
repository's own suite does.

The records mirror the CLI's JSON contract exactly, as frozen dataclasses:

```python
Report(version, ignores, errors)
IgnoreDirective(tool, scope, rules, reason, path, line, end_line, column, raw, suppressed)
Suppressed(start_line, end_line)  # end_line is None when the range runs to end-of-file
ReportError(path, message)
```

`Tool` and `Scope` are `enum.StrEnum`s, so `directive.tool == "ruff"`,
`f"{directive.tool}"`, and `"/".join([directive.scope])` all give the wire name.

Only a clean run returns. Any non-zero exit — including the 2 the CLI uses for a
file it could not read — raises `NotignoredExitError` with the CLI's own stderr,
so a tree that could not be fully scanned can never be mistaken for a clean one.
`Report.errors` is part of the v1 envelope and is carried on the record, but a
returned report always has it empty for that reason.

## Errors

Everything raised is a `NotignoredError`:

| Error | When |
| --- | --- |
| `NotignoredNotFoundError` | No `notignored` binary could be found. The message says how to install one. |
| `NotignoredSpawnError` | The binary is there but the process could not start. |
| `NotignoredExitError` | The CLI exited non-zero; carries `returncode` and the CLI's `stderr`. |
| `NotignoredContractError` | The output is not the report contract this SDK reads. |

Parsing is strict: an unknown tool, an unknown scope, or a missing field is a
`NotignoredContractError`, never a silently dropped record. Because the package
pins its CLI exactly, a supported install cannot hit it.

## Working on it

From the repository root:

```bash
just bootstrap                             # provisions every project
just nx run notignored-sdk-python:check    # this project's gate alone
just check                                 # the whole repo's gate
```
