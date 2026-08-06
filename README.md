# notignored

Find every lint and type-check suppression comment in a codebase — natively, and
fast.

```console
$ notignored src/
src/app.py:3:12 ruff F401 (line) -- re-exported for the public API
src/app.py:5:58 ruff E501 (line) -- long wrapped URL
src/app.py:10:17 ruff * (line)
src/vendored.py:1:1 ruff E501 (file) -- vendored upstream, not ours to reformat
notignored: 4 ignores in 2 files
```

## Why

Suppression comments are where lint and type-check debt hides. A `# noqa` costs
one line to add and scrolls past review unremarked — especially in agent-written
code, where silencing a rule is often easier than fixing it.

`notignored` turns every suppression into a first-class, queryable record: which
tool, which rules, the stated reason, and exactly where it lives. That makes a
high-level review of a large change possible — you can see the bypasses and their
justifications without reading every line.

It parses the directives itself and **never invokes the tool whose rule is being
silenced**, so scanning a tree costs a read and a scan instead of ten linter
startups. That is what makes it cheap enough to run on every pull request.

## Install

```console
# Cross-platform, from source (Linux, macOS, Windows):
cargo install --git https://github.com/nickderobertis/notignored --locked

# Or a prebuilt binary (Linux, macOS, and Windows under a POSIX shell):
curl -fsSL https://raw.githubusercontent.com/nickderobertis/notignored/main/scripts/install.sh | sh
```

The installer honours `NOTIGNORED_VERSION` / `NOTIGNORED_INSTALL_DIR` (or the
`--version` / `--to` flags), verifies the archive against the SHA-256 checksum
published beside it, and refuses to install a binary it cannot verify. Every
tagged release attaches per-platform archives built on native runners.

## Usage

```
notignored [PATHS...] [--format human|json] [--tool NAME]... [--fail-if-found]
           [--diff [--diff-base REF]]
```

- `PATHS` — files and/or directories. Directories are walked recursively,
  honouring `.gitignore`. Defaults to `.`.
- `--format` — `human` (default) or `json`.
- `--tool` — only report this tool; repeat to allow several. Omit for all.
- `--fail-if-found` — exit 1 when any suppression is reported.
- `--diff` — report only the suppressions the change added (see below).
- `--diff-base` — the git revision or range `--diff` compares against.

### Reviewing a pull request

A reviewer cares about the suppressions a change *introduces*, not the
inventory it inherited. `--diff` reports only those: a directive is new when the
diff added at least one of the lines it occupies.

```console
$ notignored --diff --diff-base main --fail-if-found
src/app.py:42:20 ruff E501 (line) -- long wrapped URL
notignored: 1 ignore in 1 file
```

- Bare `--diff` compares the work tree — staged *and* unstaged — against `HEAD`.
- `--diff-base REF` takes any git revision or range. A **plain ref** is compared
  from the **merge base**, the way a pull request's "Files changed" is, so
  commits that landed on the base branch after this one forked are never
  reported as this branch's own. An explicit `A..B` **range** is passed to git
  as-is, two-dot semantics and all. These are llmlint's `--diff` / `--diff-base`
  semantics exactly.
- `PATHS` still narrow the result: `notignored --diff --diff-base main src/`
  reports the new suppressions under `src/` only. An empty intersection is a
  clean exit 0.
- Only the files the change touched are read, so a diff run stays fast on a
  large repository. Files the change deleted are skipped; a renamed file reports
  what the change added to it, not the lines that merely moved.

`--diff` shells out to `git` — infrastructure, not one of the linters whose
directives are parsed natively — so it needs `git` on `PATH` and a work tree.

### Exit codes

| Code | Meaning |
| --- | --- |
| `0` | The scan completed. |
| `1` | `--fail-if-found` was given and at least one suppression was reported. |
| `2` | The scan could not complete — an unreadable path or file, or a bad argument. |

Findings go to stdout; the summary and any errors go to stderr, so
`notignored --format json > report.json` captures only the report. A downstream
consumer that stops reading (`| head`, `| grep -q`) is not an error: the scan's
own verdict still decides the exit code.

## Output

The `json` format emits the full report envelope:

```json
{
  "version": 1,
  "ignores": [
    {
      "tool": "ruff",
      "scope": "line",
      "rules": ["E501"],
      "reason": "long wrapped URL",
      "path": "src/app.py",
      "line": 12,
      "end_line": 12,
      "column": 20,
      "raw": "# noqa: E501  # long wrapped URL",
      "suppressed": { "start_line": 12, "end_line": 12 }
    }
  ],
  "errors": []
}
```

- `scope` — `line`, `next-line`, `file`, or `block`.
- `rules` — rule names/codes exactly as written. `[]` means a blanket
  suppression of every rule the tool would apply (rendered as `*` in the human
  format).
- `reason` — the stated justification, or `null`. Taken from the tool's native
  reason syntax where one exists, otherwise from the trailing comment on the
  directive line; whitespace is collapsed to single spaces.
- `line` / `end_line` / `column` — 1-based.
- `suppressed` — the best-effort range the directive silences. `end_line` is
  `null` when it runs to end-of-file or is unterminated.
- `errors` — files that could not be read. Never a panic.

`version` is the envelope version; it changes only when the shape does.

## Supported tools

| Tool | Directives | Status |
| --- | --- | --- |
| `eslint` | `// eslint-disable-next-line rule -- reason` | Planned |
| `biome` | `// biome-ignore lint/group/rule: reason` | Planned |
| `ruff` | `# noqa`, `# noqa: E501, F401`, `# ruff: noqa`, `# ruff: noqa: E501` | **Supported** |
| `typescript` | `// @ts-ignore`, `// @ts-expect-error` | Planned |
| `mypy` | `# type: ignore`, `# type: ignore[arg-type, index]`, `# mypy: ignore-errors`, `# mypy: disable-error-code="arg-type"` | **Supported** |
| `pyright` | `# pyright: ignore`, `# pyright: ignore[reportArgumentType]` | **Supported** |
| `ty` | `# ty: ignore`, `# ty: ignore[invalid-argument-type]` | **Supported** |
| `rust` | `#[allow(dead_code)]`, `#[allow(clippy::needless_collect, dead_code)]`, `#[expect(dead_code, reason = "…")]`, `#![allow(…)]`, `#![expect(…, reason = "…")]` | **Supported** |
| `shellcheck` | `# shellcheck disable=SC2086`, `# shellcheck disable=SC2086,SC2046`, `# shellcheck disable=SC2000-SC2100`, `# shellcheck disable=all`, `# shellcheck disable=SC2086  # reason` | **Supported** |
| `llmlint` | `ignore[rule, …] reason`, `ignore-file[rule, …] reason`, `ignore-block[rule, …] reason` … `ignore-end[rule, …]` — each written after the `llmlint` keyword and a colon, in the host language's comment syntax | **Supported** |

Scope follows each tool's own rules, not a house convention:

- **rust** — an outer attribute is `next-line` and its `suppressed` range runs
  through the end of the item it annotates; an inner `#![…]` is `file`. A
  `reason = "…"` is the record's reason, even when the string wraps.
- **shellcheck** — a directive above the first command is `file`; anywhere else
  it is `next-line`. A directive ShellCheck itself rejects (trailing prose with
  no `#`, or one placed after a command) is reported by neither tool.
- **llmlint** — `ignore` is `line`, `ignore-file` is `file`, and
  `ignore-block` … `ignore-end` is one `block` record spanning both directives.
  A block left unclosed keeps a null `suppressed.end_line` and adds an `errors`
  entry.

Adding one is three touch points: a module under `src/tools/`, one line in
`src/tools/mod.rs::registry()`, and one row above. The registry is the single
source of truth for the Status column: `tests/tools_contract.rs` fails the build
if a row here and the registered parsers disagree.

## Where a directive reaches, and who honours it

Every `# type: ignore` and `# pyright: ignore` is `line`-scoped; mypy's
module-wide exemptions are the two `# mypy:` config comments; and for ty, where
the comment sits is the scope:

| Source | Reported as |
| --- | --- |
| `f(x)  # type: ignore` | `mypy`, `line` |
| `# mypy: ignore-errors` on its own line | `mypy`, `file` |
| `# mypy: disable-error-code="arg-type"` on its own line | `mypy`, `file` |
| `f(x)  # pyright: ignore` | `pyright`, `line` |
| `f(x)  # ty: ignore` | `ty`, `line` |
| `# ty: ignore` above every statement | `ty`, `file` |
| `# ty: ignore` on its own line in the body | `ty`, `next-line` |

Several tools honour a directive they did not invent: pyright and ty both act on
mypy's `# type: ignore`, and ruff, pyright, and ty all act on one that does not
open its comment (mypy does not). A directive is reported **once, under the tool
whose syntax it is** — `# type: ignore` is one `mypy` record, not three — so a
count of records is a count of suppressions written, not of checkers affected.

One line can still carry directives for several tools, and each record covers
**its own directive only**. Given

```python
import legacy  # type: ignore[import-not-found]  # no stubs published  # noqa: F401  # imported for its side effects
```

the `mypy` record's `raw` stops at `# no stubs published` and its `reason` is
`"no stubs published"`; the `ruff` record's `raw` starts at `# noqa: F401` and
its `reason` is `"imported for its side effects"`. A record's `raw` and `reason`
always end where the next tool's directive begins, so one tool's live suppression
can never be filed as another's justification.

`# pyright: basic` and `# pyright: strict` switch pyright's type-checking mode
rather than silencing a diagnostic, and are deliberately not reported.

## How it works

Source is scanned once per file by a language-aware comment extractor
(`src/comments.rs`) that understands `#` comments, `//` line comments, multi-line
`/* … */` blocks (nested, for Rust), Rust attributes, and the punctuation that
delimits a Rust item — and that knows a string literal when it sees one, so
`MESSAGE = "# noqa: E501"` is never reported. Tool parsers consume that
extraction; they never re-scan raw lines.

## Development

```console
just bootstrap   # from a clean clone
just check       # the full gate: format, clippy, tests + coverage, docs
just --list      # everything else
```

`just check` runs the end-to-end suite, which drives the compiled binary as a
subprocess and the **real, pinned** tools — `ruff`, `mypy`, `pyright`, `ty`,
`shellcheck`, and `llmlint` (see the `.<tool>-version` files) plus the pinned
toolchain's own `rustc` and `clippy-driver` — to prove that what `notignored`
reports is what those tools actually suppress. `just bootstrap` installs the
Python-packaged ones with `uv` into project-local venvs under `.dev/`.

## License

MIT — see [LICENSE](LICENSE).
