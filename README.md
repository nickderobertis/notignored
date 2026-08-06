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
```

- `PATHS` — files and/or directories. Directories are walked recursively,
  honouring `.gitignore`. Defaults to `.`.
- `--format` — `human` (default) or `json`.
- `--tool` — only report this tool; repeat to allow several. Omit for all.
- `--fail-if-found` — exit 1 when any suppression is reported.

### Exit codes

| Code | Meaning |
| --- | --- |
| `0` | The scan completed. |
| `1` | `--fail-if-found` was given and at least one suppression was reported. |
| `2` | The scan could not complete — an unreadable path or file, or a bad argument. |

Findings go to stdout; the summary and any errors go to stderr, so
`notignored --format json > report.json` captures only the report.

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
| `mypy` | `# type: ignore`, `# type: ignore[arg-type]` | Planned |
| `pyright` | `# pyright: ignore[reportAny]` | Planned |
| `ty` | `# ty: ignore[unresolved-import]` | Planned |
| `rust` | `#[allow(…)]`, `#[expect(…, reason = "…")]` | Planned |
| `shellcheck` | `# shellcheck disable=SC2086` | Planned |
| `llmlint` | `llmlint: ignore[rule] reason` | Planned |

Adding one is three touch points: a module under `src/tools/`, one line in
`src/tools/mod.rs::registry()`, and one row above.

## How it works

Source is scanned once per file by a language-aware comment extractor
(`src/comments.rs`) that understands `#` comments, `//` line comments, multi-line
`/* … */` blocks (nested, for Rust), and Rust attributes — and that knows a string
literal when it sees one, so `MESSAGE = "# noqa: E501"` is never reported. Tool
parsers consume those extracted comments; they never re-scan raw lines.

## Development

```console
just bootstrap   # from a clean clone
just check       # the full gate: format, clippy, tests + coverage, docs
just --list      # everything else
```

`just check` runs the end-to-end suite, which drives the compiled binary as a
subprocess and the **real, pinned** `ruff` (see `.ruff-version`) to prove that
what `notignored` reports is what `ruff` actually suppresses.

## License

MIT — see [LICENSE](LICENSE).
