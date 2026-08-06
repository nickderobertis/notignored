# Canonical command surface for notignored.
#
# `just bootstrap` works from a clean clone; `just check` is the full quality
# gate and fails on any issue (no warnings-only mode). Recipes are quiet on
# success and specific on failure.

set shell := ["bash", "-eu", "-o", "pipefail", "-c"]

# Pinned cargo dev tools the gate drives but the toolchain doesn't ship.
# `just bootstrap` installs these; CI installs the latest via
# taiki-e/install-action. Keep in sync with .github/workflows/ci.yml.
nextest-version := "0.9.140"
llvmcov-version := "0.8.7"

# List available recipes.
default:
    @just --list

# Installs toolchain components, the pinned cargo dev tools, deps, and the
# pinned ruff the e2e parity suite drives.
# Set up the project from a clean clone.
bootstrap:
    rustup show active-toolchain
    rustup component add rustfmt clippy llvm-tools
    @just _ensure-tool cargo-nextest {{nextest-version}}
    @just _ensure-tool cargo-llvm-cov {{llvmcov-version}}
    cargo fetch --locked
    @bash scripts/setup-ruff.sh

# Install a pinned cargo tool if it is missing. Quiet when already present.
_ensure-tool tool version:
    @command -v {{tool}} >/dev/null 2>&1 \
      || cargo install {{tool}} --version {{version}} --locked

# Format check, lint, tests (unit + integration + e2e) with coverage enforced,
# and docs. Fails on any issue; no warnings-only mode.
# Full quality gate.
check: fmt-check lint test doc
    @echo "check: ok"

# Verify formatting without modifying files.
fmt-check:
    cargo fmt --all -- --check

# Format the codebase in place.
format:
    cargo fmt --all

# Lint with clippy; any warning is an error.
lint:
    cargo clippy --all-targets --locked -- -D warnings

# 95% line coverage is the gate; lower it only with a documented reason in
# AGENTS.md.
# Full test suite (unit + integration + e2e) with coverage enforced.
test:
    cargo llvm-cov nextest --locked --fail-under-lines 95

# Drives the compiled binary and the real pinned ruff — never a stub.
# The end-to-end binary journeys in isolation (also run by `test`/`check`).
test-e2e:
    cargo nextest run --locked -E 'binary(e2e)'

# Build the docs with warnings denied (kept in the gate so doc links don't rot).
doc:
    RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --locked

# Run the CLI against a path, e.g. `just run src/ --format json`.
run *ARGS:
    cargo run --locked --quiet -- {{ARGS}}

# Install the pinned ruff the e2e parity suite drives (also run by `bootstrap`).
setup-ruff:
    @bash scripts/setup-ruff.sh

# Only after reviewing the diff — and bump REPORT_VERSION when the shape, not
# just the data, moved.
# Rewrite the checked-in golden report from the current output.
bless:
    NOTIGNORED_BLESS=1 cargo test --locked --test e2e json_format

# Upgrade dependencies, then re-run the full gate.
upgrade:
    cargo update
    @just check

# Separate from `check`: `cargo deny` needs a network-fetched advisory DB.
# Advisory + license audit and unused-dependency check.
deps-check:
    @command -v cargo-deny >/dev/null || { echo "cargo-deny not installed: cargo install cargo-deny --locked" >&2; exit 1; }
    @command -v cargo-machete >/dev/null || { echo "cargo-machete not installed: cargo install cargo-machete --locked" >&2; exit 1; }
    cargo deny check
    cargo machete

# Build under the declared MSRV (needs the 1.85 toolchain installed).
msrv:
    cargo +1.85 check --locked --all-targets

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
    llmlint --diff --diff-base "{{base}}" {{args}}
