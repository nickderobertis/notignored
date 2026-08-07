# Terminal screenshots

Deterministic SVG screenshots of notignored's **real** colorized output, gated by
[screencomp](https://github.com/nickderobertis/screencomp). Informational —
**never part of `just check`, `just bootstrap`, or the CI gate**; the `Visual
docs` workflow (`.github/workflows/visual-docs.yml`) owns the comparison on PRs.

## What it is

`scripts/screenshots.sh` drives the **real release `notignored` binary** over the
committed fixture tree in `fixture/` — exactly as the e2e suite drives it — so
every captured character is genuine CLI output. Nothing is scripted, mocked, or
hand-written. Each scene's output is rendered to an SVG by
[`freeze`](https://github.com/charmbracelet/freeze).

One scene per part of the CLI surface, so the gallery documents all of it:

- `scan` — the default human report over the whole fixture. Colorized (real ANSI
  through `--color always`), which is the point of the scene: location, tool,
  rules, scope, and reason each have their own role, and the fixture's blanket
  `*` renders in the alarm colour.
- `diff` — the review case. `--diff` needs a repository, so the script builds one:
  a throwaway copy of `fixture/`, committed as the base, with the `change/`
  overlay laid on top as the uncommitted work under review. Bare `--diff` then
  compares the work tree against `HEAD`, so only what the change added is
  reported. Colorized.
- `tool-filter` — `--tool`, repeated, narrowing that same scan.
- `json` — the full report envelope. Scoped to **one** fixture file on purpose:
  the envelope is what the scene documents, and a whole-tree capture would run
  off the bottom of the frame with the same fields repeated seven times.
- `pr-comment` — `--format markdown` over the same change: the exact body the
  GitHub Action posts, permalinks and all.

## The fixture

`fixture/` is a tiny four-file project carrying the kind of suppression, and the
kind of reason, real code collects — seven directives across seven tools (ruff,
mypy, eslint, typescript, rust, shellcheck, llmlint). It is deliberately not
uniform:

- `src/retry.rs` holds a `reason = "…"` that **wraps across two lines** of the
  attribute, so the gallery shows a multi-line reason collapsing into one.
- `src/widget.ts`'s `@ts-expect-error` is a **blanket** suppression — it names no
  rules, so it renders as `*`.
- `scripts/deploy.sh`'s `# shellcheck disable=SC2086` has **no reason at all**,
  which is the finding this tool exists to surface. That is what its
  `llmlint: ignore-file[suppressions_justified]` footer buys, per the repo
  `AGENTS.md` convention — and the footer is itself a directive we parse, so it
  shows up in the report like any other.

`change/` holds the files the `diff` scene overwrites the base tree with. Keep
its additions **appended**, not interleaved: `--diff` reports a directive when the
change added a line it occupies, so inserting above an existing directive would
report that one too and the scene would stop being about new suppressions.

**Consistent text size.** Every scene renders at a fixed window width (`freeze
--width 919`, with `--wrap 102` folding the few over-wide lines — the markdown
permalinks, mostly), so the gallery and README — which display each SVG at one
fixed width — render the text at the same size on every card. Without this,
auto-width made a narrow `tool-filter` scale up huge next to a tall `json`.

**Nothing to normalize.** Every scene runs with the fixture (or the throwaway
repository) as its working directory, and the report names paths relative to
that, so no absolute path reaches the capture. The one value that could not be
real is the `pr-comment` scene's commit sha — the sha of the commit that adds a
screenshot is not knowable while taking it — so it is a fixed literal in the
script. The scripted git history is built with `GIT_CONFIG_GLOBAL` and
`GIT_CONFIG_SYSTEM` pointed at `/dev/null` and a fixed identity, so a
developer's own gitconfig (signing, hooks, `autocrlf`, a default branch name)
cannot change what is captured.

## Why it is byte-reproducible (and needs no container)

screencomp gates on the **hash** of each image, so capture must be deterministic.
Unlike a rasterized PNG (whose anti-aliasing drifts across CPUs), an SVG is pure
layout math. We pin both inputs:

- **`freeze` is version-pinned** — `just`'s `freeze-version`, CI's
  `capture-command`, and `screenshots-tools` all agree, and
  `tests/screenshots_contract.rs` fails the build if they ever stop agreeing.
- **The font is vendored** (`fonts/JetBrainsMono-Regular.ttf`, OFL — see
  `fonts/JetBrainsMono-OFL.txt`) and passed via `--font.file`, so freeze never
  fetches one over the network (which also makes capture offline and fast). It is
  embedded into each SVG as base64, so the file renders the same on GitHub with
  nothing external to load.

The result: identical bytes on every machine and runner, so a single `x86_64`
lane and baseline cover everyone — an SVG only changes when the report's
**content or formatting** changes, which is exactly what the gate should catch.

## Outputs

- `shots/current/<arch>/captures.json` + the SVGs — the capture screencomp reads
  (gitignored; regenerated). `$SHOTS_OUT` overrides the directory; the reusable
  workflow exports it per arch lane.
- `shots/baseline/<arch>.json` — the committed digest baseline (no images).
- `docs/screenshots/*.svg` — the committed copies embedded in the README.

## The animated demo GIF (`docs/screenshots/demo.gif`)

The SVGs are static; the README **hero** is an animated GIF of a short session —
a scan command being typed, the colorized findings appearing, then a `--diff`
run. `scripts/demo-gif.py` drives the **same real release binary** over the same
`fixture/` for its data, then reconstructs the frames a terminal would draw and
renders them with the same **vendored JetBrains Mono font** (Pillow only — no
PTY recording, no `ttyd`/`ffmpeg`, no network). Unlike the SVGs it is **not**
hash-gated (a GIF is not byte-reproducible across Pillow versions), so it is
regenerated on demand (`just screenshots-gif`) and committed. Regenerate it when
the human report's format changes.

## Commands

- `just screenshots-tools` — install the pinned `freeze` (needs Go). screencomp
  is installed separately (see its README); CI installs both itself.
- `just screenshots` — capture (builds the release binary, writes the shots + the
  README copies). Quiet on success.
- `just screenshots-gif` — regenerate the animated demo GIF (needs Python 3 +
  Pillow).
- `just screenshots-bless` — after an **intended** output change, recapture and
  refresh `shots/baseline/<arch>.json`. Commit it alongside `docs/screenshots/`.

## The strict gate

CI (`fail-on-drift: true`) fails when a capture diverges from the committed
baseline. That job cannot be a required check — its report lane needs write
permissions a fork's pull request is never granted — so read a red `Visual docs`
as "the output moved; bless it or explain it", not as advisory noise.

The local pre-push guard (`.githooks/pre-push`, opt in with
`git config core.hooksPath .githooks`) re-captures **only** when a `[guard].paths`
file changes (`screencomp.toml`), and on drift it regenerates the baseline,
builds a review gallery (`shots/review/index.html`), and blocks the push so you
commit the refreshed baseline + README images deliberately.

## Changing the screenshots

Editing either renderer (`src/cli/render.rs`, `src/cli/markdown.rs`), the CLI
surface, a parser under `src/tools/`, the fixture, or the scenes in
`scripts/screenshots.sh` will change the SVGs. That is expected — run
`just screenshots-bless` and commit the new baseline + `docs/screenshots/`.
Bumping `freeze-version` or the vendored font reflows every shot; bless once and
keep the pins in step (the contract test will tell you if you didn't).
