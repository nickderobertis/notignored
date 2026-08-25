# Canonical command surface for notignored.
#
# `just bootstrap` works from a clean clone; `just check` is the full quality
# gate and fails on any issue (no warnings-only mode). Recipes are quiet on
# success and specific on failure.
#
# This is a monorepo: the repo-wide verbs (bootstrap, check, lint, test, format,
# fmt-check, upgrade) delegate to Nx, which fans the uniformly-named target out
# across every project. They never loop over projects by hand. What a target
# *does* stays with its project — the `_crate-*` recipes below are the Rust
# crate's own tools, and the SDKs' project.json files name theirs.

set shell := ["bash", "-eu", "-o", "pipefail", "-c"]

# llmlint: ignore-file[tool_output_is_signal] recipes that hand straight to cargo,
# clippy, rustdoc, or cargo-deny inherit those tools' diagnostics, which already
# name the exact problem and its fix; a wrapper message would bury them. Recipes
# whose failure needs project-level context (_crate-bootstrap, _crate-test, msrv,
# _crate-fmt-check) add
# one explicitly.

# The MSRV has one source of truth — Cargo.toml's `rust-version` — so `just msrv`
# cannot promise a floor the manifest no longer declares. CI reads the same field.
msrv-version := `sed -n 's/^rust-version *= *"\([^"]*\)".*/\1/p' Cargo.toml`

# Renderer for the terminal screenshots (`just screenshots`). NOT part of the
# gate or `just bootstrap`: screenshots are informational. CI's Visual-docs
# workflow installs the same pinned version, and `tests/screenshots_contract.rs`
# fails the build if the two ever disagree.
freeze-version := "0.2.2"

# Keep the gate's own output to signal: successes are silent, failures are not.
export CARGO_TERM_QUIET := "true"

# List available recipes.
default:
    @just --list

# Every project's `bootstrap` target, so one clean-clone command provisions the
# whole graph rather than the crate alone.
#
# Serialized on purpose. Projects share installers — the crate and the Python SDK
# both need the pinned ruff, the crate and the TS SDK both need the pinned biome
# — and `scripts/setup-*.sh` recreate a `.dev/<tool>` tree from scratch when the
# pin moved. Two of those running at once race on the same directory and the
# second one fails on a venv the first is still filling. They are idempotent, so
# running them one at a time costs a no-op, not a reinstall.
# Set up the project from a clean clone.
bootstrap:
    @bash scripts/nx.sh run-many -t bootstrap --parallel=1

# Installs toolchain components, the pinned cargo dev tools, deps, and the
# pinned linters and type checkers the e2e parity suites drive.
# The Rust crate's own provisioning (the `notignored:bootstrap` target).
_crate-bootstrap:
    @rustup show active-toolchain >/dev/null 2>&1 || rustup toolchain install
    @rustup component add rustfmt clippy llvm-tools >/dev/null \
      || { echo "cannot add toolchain components — install rustup (https://rustup.rs/) and re-run" >&2; exit 1; }
    @just _ensure-tool cargo-nextest
    @just _ensure-tool cargo-llvm-cov
    @cargo fetch --locked --quiet
    @bash scripts/setup-python-tools.sh
    @bash scripts/setup-js.sh
    @bash scripts/setup-misc-tools.sh

# These are test runners, not rules: their version cannot change the gate's
# verdict, so both here and CI take the latest rather than keeping two pins that
# drift apart.
# Install a cargo dev tool if it is missing. Quiet when already present.
_ensure-tool tool:
    @command -v {{tool}} >/dev/null 2>&1 || cargo install {{tool}} --locked --quiet

# The tiers run in fail-fast order as dependencies, each fanned across every
# project by Nx. The body then runs the per-project `check` aggregate — the same
# target `just check-affected` and a single project's gate
# (`just nx run notignored-sdk-python:check`) use — which replays from the cache
# in a second and is what stops the full sweep and the affected sweep from ever
# covering different tiers.
# Full quality gate, every project.
check: fmt-check lint test doc
    @bash scripts/nx.sh run-many -t check
    @echo "check: ok"

# What PR CI runs: the same gate, scoped to the projects this branch's diff can
# reach. Fails closed — with no derivable merge base it runs everything.
# Full quality gate, affected projects only.
check-affected:
    @bash scripts/nx-affected.sh -t check
    @echo "check-affected: ok"

# `true` when this branch's diff can reach the Rust crate project, so CI can skip
# the cross-platform and install matrices on an SDK-only change. Fails closed.
# Whether the Rust crate is affected by this branch.
affected-crate:
    @bash scripts/nx-affected.sh --affects notignored

# Escape hatch for Nx itself, e.g. `just nx show projects` or `just nx graph`.
# Run an arbitrary Nx command against this workspace.
nx *ARGS:
    @bash scripts/nx.sh {{ARGS}}

# Verify formatting without modifying files.
fmt-check:
    @bash scripts/nx.sh run-many -t format-check

# Format the codebase in place.
format:
    @bash scripts/nx.sh run-many -t format

# Lint every project with its own linter; any warning is an error.
lint:
    @bash scripts/nx.sh run-many -t lint

# Every project's test suite; the crate's enforces its coverage floor.
test:
    @bash scripts/nx.sh run-many -t test

# Verify the crate's formatting without modifying files.
_crate-fmt-check:
    @cargo fmt --all -- --check || { echo "formatting drift above — run 'just format'" >&2; exit 1; }

# Format the crate in place.
_crate-format:
    @cargo fmt --all

# Lint the crate with clippy; any warning is an error.
_crate-lint:
    @cargo clippy --all-targets --locked --quiet -- -D warnings

# 95% line coverage is the gate; lower it only with a documented reason in
# AGENTS.md.
# The crate's full test suite (unit + integration + e2e) with coverage enforced.
_crate-test:
    @cargo llvm-cov nextest --locked --fail-under-lines 95 \
      --status-level fail --final-status-level fail \
      || { echo "tests failed, or coverage fell below 95% — cover the lines the table above counts as missed" >&2; exit 1; }

# Coverage instrumentation is measured on Linux only, so the cross-platform CI
# legs run the same suite through this instead of `test`.
# Full test suite without coverage instrumentation.
test-quick:
    @cargo nextest run --locked --status-level fail

# Drives the compiled binary and the real pinned linters — never a stub.
# The end-to-end binary journeys in isolation (also run by `test`/`check`).
test-e2e:
    @cargo nextest run --locked -E 'binary(e2e)' --status-level fail

# Build the docs with warnings denied (kept in the gate so doc links don't rot).
doc:
    @RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --locked --quiet

# Run the CLI against a path, e.g. `just run src/ --format json`.
run *ARGS:
    cargo run --locked --quiet -- {{ARGS}}

# Also run by `bootstrap`; this is the manual entry point. A recipe's doc is only
# its LAST comment line, so the one-liners below have to stay one line.
# Install the pinned ruff/mypy/pyright/ty the e2e parity suites drive.
setup-python-tools:
    @bash scripts/setup-python-tools.sh

# Also run by `bootstrap`; this is the manual entry point.
# Install the pinned eslint/biome/tsc the e2e parity suites drive.
setup-js:
    @bash scripts/setup-js.sh

# Install the pinned shellcheck/llmlint the e2e parity suites drive (also run by
# `bootstrap`). Rust needs no pin here — `rust-toolchain.toml` is the one source.
setup-misc-tools:
    @bash scripts/setup-misc-tools.sh

# Only after reviewing the diff — and bump REPORT_VERSION when the shape, not
# just the data, moved.
# Rewrite the checked-in golden reports from the current output.
bless:
    @NOTIGNORED_BLESS=1 cargo test --quiet --locked --test e2e golden

# Upgrade dependencies, then re-run the full gate.
upgrade:
    @cargo update --quiet
    @npm update --silent --no-audit --no-fund
    @just check

# --- Terminal screenshots (informational; never part of `check` or CI's gate) -
# Deterministic SVGs of the real CLI output, rendered by `freeze` from a vendored
# pinned font, gated/galleried/PR-commented by screencomp (see
# screenshots/AGENTS.md). Regenerating is out of the gate on purpose: `check`
# stays offline and toolchain-only, and CI's Visual-docs workflow owns the
# comparison.

# Install the pinned screenshot renderer (`freeze`) on demand. Needs Go.
screenshots-tools:
    @command -v go >/dev/null || { echo "go not found: needed to install freeze; see https://go.dev/dl" >&2; exit 1; }
    go install github.com/charmbracelet/freeze@v{{freeze-version}}
    @echo "installed freeze to $(go env GOPATH)/bin (ensure it is on PATH)"

# Capture the screenshots: drive the real release binary over the fixture, render
# each scene to shots/current/<arch>/ + docs/screenshots/. Needs `freeze` on PATH.
screenshots:
    @bash scripts/screenshots.sh

# Regenerate the animated demo GIF (docs/screenshots/demo.gif — the README hero).
# Like the screenshots it drives the REAL release binary over the fixture, then
# renders the frames a session would show with the vendored font (Pillow only —
# no PTY recording, no ffmpeg). Informational and NOT hash-gated (a GIF is not
# byte-reproducible across Pillow versions), so regenerate on demand and commit
# the result. Needs Python 3 + Pillow (`pip install Pillow`).
screenshots-gif:
    @command -v python3 >/dev/null || { echo "python3 not found: needed to render the demo GIF" >&2; exit 1; }
    @python3 -c "import PIL" 2>/dev/null || { echo "Pillow not installed: pip install Pillow" >&2; exit 1; }
    @cargo build --release --locked --bin notignored
    @python3 scripts/demo-gif.py

# llmlint: ignore-block[changed_behavior_has_e2e] a test of either success path
# would have to install the browser these deliberately keep out of the gate.
# Also run on demand by `screenshots-pr-comment`; this is the manual entry point.
# Install the pinned markdown/highlighting/browser toolchain (needs Node.js 20+).
screenshots-comment-tools:
    @bash scripts/setup-comment-render.sh

# NOT hash-gated, so nothing notices when it goes stale — screenshots/AGENTS.md
# says when to re-run it.
# Re-photograph the README's comment: the real binary's body, light and dark.
screenshots-pr-comment:
    @command -v node >/dev/null || { echo "node not found: the render needs Node.js 20+, see https://nodejs.org" >&2; exit 1; }
    @bash scripts/setup-comment-render.sh
    @cargo build --release --locked --bin notignored
    @bash scripts/pr-comment-body.sh | node scripts/comment-render/render.mjs \
      docs/screenshots/pr-comment-rendered.png docs/screenshots/pr-comment-rendered-dark.png

# llmlint: ignore-end[changed_behavior_has_e2e]

# Refresh the committed baseline manifest from a fresh capture (after an
# INTENDED output change). Commit shots/baseline/ + docs/screenshots/ alongside.
screenshots-bless: screenshots
    @command -v screencomp >/dev/null || { echo "screencomp not installed: https://github.com/nickderobertis/screencomp#install" >&2; exit 1; }
    screencomp manifest --input shots/current --output shots/baseline/$(uname -m | sed 's/amd64/x86_64/;s/aarch64/arm64/').json
    @echo "baseline refreshed; commit shots/baseline/ + docs/screenshots/"

# Separate from `check`: `cargo deny` needs a network-fetched advisory DB.
# Advisory + license audit and unused-dependency check.
deps-check:
    @command -v cargo-deny >/dev/null || { echo "cargo-deny not installed: cargo install cargo-deny --locked" >&2; exit 1; }
    @command -v cargo-machete >/dev/null || { echo "cargo-machete not installed: cargo install cargo-machete --locked" >&2; exit 1; }
    @cargo deny --log-level error check
    @# machete prints the unused deps it finds on stdout, so keep it: hiding
    @# them would leave a failing gate with no actionable detail.
    @cargo machete

# Reads the floor from Cargo.toml's `rust-version`; that toolchain must be
# installed (`rustup toolchain install <version>`). Warnings are errors here too.
# Build under the declared MSRV.
msrv:
    @RUSTFLAGS="-D warnings" cargo +{{msrv-version}} check --locked --all-targets --quiet \
      || { echo "the {{msrv-version}} floor no longer builds — install that toolchain, or raise rust-version in Cargo.toml (and clippy.toml)" >&2; exit 1; }

# Ensures `just`, verifies the rest, then runs setup-llmlint. Runs automatically
# via the Claude Code SessionStart hook; this is the manual entry point.
# Provision the dev toolchain for a session. Idempotent, no-ops in CI.
session-setup:
    ./scripts/session-setup.sh

# Install/refresh the llmlint toolchain (oneharness + llmlint). Idempotent.
setup-llmlint:
    ./scripts/setup-llmlint.sh

# Kept OUT of `check` on purpose: the gate stays deterministic and offline.
# Config is the composed `llmlint.yml`.
# LLM-judge lint — the non-deterministic, harness-backed tier.
lint-llm *paths:
    @command -v llmlint >/dev/null 2>&1 || { echo "llmlint not installed — run 'just setup-llmlint'"; exit 1; }
    llmlint {{paths}}

# CI runs this before the model tier so a broken config fails in milliseconds
# instead of spending a harness call.
# Fast, deterministic llmlint gate — no model calls, no harness credential.
lint-llm-validate *args:
    @command -v llmlint >/dev/null 2>&1 || { echo "llmlint not installed — run 'just setup-llmlint'"; exit 1; }
    llmlint validate {{args}}

# The blocking `llmlint` PR check; run it locally before pushing.
# llmlint scoped to the files this branch changed since it forked from main.
lint-llm-diff base="origin/main" *args:
    @command -v llmlint >/dev/null 2>&1 || { echo "llmlint not installed — run 'just setup-llmlint'"; exit 1; }
    llmlint --diff --diff-base "{{base}}" {{args}}
