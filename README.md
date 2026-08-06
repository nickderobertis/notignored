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

In CI there is nothing to install: the [GitHub Action](#on-a-pull-request-the-github-action)
fetches the release binary itself and posts what the pull request added.

```yaml
- uses: nickderobertis/notignored@main
```

## Usage

```
notignored [PATHS...] [--format human|json|markdown] [--tool NAME]... [--fail-if-found]
           [--diff [--diff-base REF]] [--github-repo OWNER/REPO] [--github-sha SHA]
```

- `PATHS` — files and/or directories. Directories are walked recursively,
  honouring `.gitignore`. Defaults to `.`.
- `--format` — `human` (default), `json`, or `markdown` (a pull-request comment
  body; see the action below).
- `--tool` — only report this tool; repeat to allow several. Omit for all.
- `--fail-if-found` — exit 1 when any suppression is reported.
- `--diff` — report only the suppressions the change added (see below).
- `--diff-base` — the git revision or range `--diff` compares against.
- `--github-repo` / `--github-sha` — the `owner/repo` and commit the `markdown`
  format builds its permalinks from.

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
- Git names a file in bytes and a report names it with a string, so a path that
  is not valid UTF-8 has no faithful spelling here. It becomes an `errors` entry
  (and exit 2) rather than a file quietly dropped from the review — the lossy
  spelling would name a file that does not exist.

`--diff` shells out to `git` — infrastructure, not one of the linters whose
directives are parsed natively — so it needs `git` on `PATH` and a work tree.

### On a pull request: the GitHub Action

The action posts one sticky comment naming every suppression the pull request
added, with its stated reason and a link to the line. It edits that same comment
on each push instead of adding another, and on a pull request that adds none it
posts nothing at all.

```yaml
# .github/workflows/notignored.yml
name: notignored

on:
  pull_request:

permissions:
  contents: read
  pull-requests: write

jobs:
  suppressions:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
        with:
          fetch-depth: 0 # the base branch has to be fetched to diff against it
      - uses: nickderobertis/notignored@main
```

| Input | Default | Meaning |
| --- | --- | --- |
| `github-token` | `${{ github.token }}` | Token used to upsert the comment. Needs `pull-requests: write`. |
| `diff-base` | the pull request's base branch | Any git revision or range, as `--diff-base` takes. |
| `paths` | the whole repository | Whitespace-separated files and directories to scan. |
| `version` | `latest` | A release tag such as `v0.1.0`, or `local` to build the action's own source with `cargo`. |

It exposes `count` (how many suppressions the change added) and `report-path`
(the JSON report), so a later step can fail the build, upload the report, or
gate on a threshold:

```yaml
      - uses: nickderobertis/notignored@main
        id: notignored
      - if: steps.notignored.outputs.count != '0'
        run: echo "this change adds ${{ steps.notignored.outputs.count }} suppression(s)"
```

The steps are `bash`, so the action runs on the Linux and macOS runners; it needs
`jq` and the `gh` CLI (both preinstalled there), plus `cargo` when
`version: local`.

`--format markdown` renders exactly the body the action posts, so it can be
previewed locally:

```console
$ notignored --diff --diff-base main --format markdown \
    --github-repo nickderobertis/notignored --github-sha "$(git rev-parse HEAD)"
```

Both permalink flags are optional; without them each location renders as plain
`path:line` text. When a body names fewer than four suppressions, each one also
carries the source line with two lines of context on either side.

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
| `eslint` | `// eslint-disable-line rule`, `// eslint-disable-next-line rule -- reason`, `/* eslint-disable rule -- reason */` … `/* eslint-enable rule */` | **Supported** |
| `biome` | `// biome-ignore lint/group/rule: reason`, `// biome-ignore-all lint/group/rule: reason`, `// biome-ignore-start lint/group/rule: reason` … `// biome-ignore-end lint/group/rule: reason` | **Supported** |
| `ruff` | `# noqa`, `# noqa: E501, F401`, `# ruff: noqa`, `# ruff: noqa: E501` | **Supported** |
| `typescript` | `// @ts-ignore`, `// @ts-expect-error reason`, `/* @ts-ignore */`, `// @ts-nocheck` | **Supported** |
| `mypy` | `# type: ignore`, `# type: ignore[arg-type, index]`, `# mypy: ignore-errors`, `# mypy: disable-error-code="arg-type"` | **Supported** |
| `pyright` | `# pyright: ignore`, `# pyright: ignore[reportArgumentType]`, `# pyright: reportMissingImports=false` | **Supported** |
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
- **typescript** — the parity claim is pinned to one compiler: the `typescript`
  version in `tests/js-toolchain/package.json` — **7.0.2**, the Go port — which
  is what `tests/e2e/typescript_parity.rs` drives. The 5.x compiler is a
  separate implementation of the same directives and is not guaranteed to read
  every form the same way, so the claim here is parity with the pinned compiler
  rather than with every `tsc` ever shipped.
- **llmlint** — `ignore` is `line`, `ignore-file` is `file`, and
  `ignore-block` … `ignore-end` is one `block` record spanning both directives.
  A block left unclosed keeps a null `suppressed.end_line` and adds an `errors`
  entry.

Each tool's own reason syntax is what gets captured: ESLint's ` -- description`,
Biome's mandatory `: explanation`, ruff's trailing `# comment`, and — for
TypeScript, which defines no separator — whatever text trails the directive. A
directive that lists no rules is a blanket suppression (`rules: []`).

Scope follows the tool rather than the syntax. `// eslint-disable-line` is
`line`; `// eslint-disable-next-line`, `// biome-ignore` and
`// @ts-expect-error` are `next-line`; `# ruff: noqa`, `// biome-ignore-all` and
`// @ts-nocheck` are `file`; and the delimited pairs
(`/* eslint-disable */` … `/* eslint-enable */`,
`// biome-ignore-start` … `// biome-ignore-end`) are `block`, running to
end-of-file with `suppressed.end_line: null` when they are never closed.

Adding one is three touch points: a module under `src/tools/`, one line in
`src/tools/mod.rs::registry()`, and one row above. The registry is the single
source of truth for the Status column: `tests/tools_contract.rs` fails the build
if a row here and the registered parsers disagree.

## Where a directive reaches, and who honours it

Every `# type: ignore` and `# pyright: ignore` is `line`-scoped; the module-wide
forms are mypy's two `# mypy:` config comments and pyright's rule override; for
ty, where the comment sits is the scope; and a Rust attribute reaches to the end
of the item it annotates:

| Source | Reported as |
| --- | --- |
| `f(x)  # type: ignore` | `mypy`, `line` |
| `# mypy: ignore-errors` on its own line | `mypy`, `file` |
| `# mypy: disable-error-code="arg-type"` on its own line | `mypy`, `file` |
| `f(x)  # pyright: ignore` | `pyright`, `line` |
| `# pyright: reportMissingImports=false` | `pyright`, `file` |
| `f(x)  # ty: ignore` | `ty`, `line` |
| `# ty: ignore` above every statement | `ty`, `file` |
| `# ty: ignore` on its own line in the body | `ty`, `next-line` |
| `#[allow(dead_code)]` above an item | `rust`, `next-line`, `suppressed` through the item's last line |
| `#[expect(dead_code, reason = "…")]` above an item | `rust`, `next-line`, `reason` from the attribute |
| `#![allow(dead_code)]` at the top of the file | `rust`, `file` |

A Rust attribute's `scope` is `next-line` — that is where the item it annotates
starts, and where a reviewer has to look — while its `suppressed` range covers
the whole item, however many lines that item runs to. An inner `#![…]` attribute
exempts the file it opens.

Pyright's `<rule>=<value>` override is reported only for the two values that
switch a rule off, `false` and `none`; `true`, `error`, `warning`, and
`information` turn a rule on or move its severity, so they are configuration
rather than suppression. Pyright reads the rest of that line as its own item
list, so the form can carry no reason — and a comment it refuses (a trailing
`# why`, a value outside those six, or a directive that does not open the
comment) silences nothing and is not reported.

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
`shellcheck`, and `llmlint` (see the `.<tool>-version` files), `eslint` /
`biome` / `tsc` (see `tests/js-toolchain/package.json`), plus the pinned
toolchain's own `rustc` and `clippy-driver` — to prove that what `notignored`
reports is what those tools actually suppress. `just bootstrap` installs them
all under `.dev/`; it needs `uv` and Node.js 20+ on `PATH`.

## License

MIT — see [LICENSE](LICENSE).
