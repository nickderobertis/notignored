# AGENTS.md

Durable instructions for humans and agents working in this repo. Write for a
future maintainer, not as a session log. Put deterministic steps in scripts and
keep this file for constraints, tradeoffs, and judgment.

> Keep this file terse — it is always-loaded context. Add a line only if a future
> task needs it **and** wouldn't surface it anyway (a failing gate, `just --list`,
> the code, or a linked doc).

> `CLAUDE.md` is a symlink to this file so the two never drift. Edit `AGENTS.md`.

## What this repo is

`notignored` is a Rust library + CLI that extracts lint/type-check suppression
comments (`# noqa`, `// eslint-disable-next-line`, `#[allow(...)]`, …) from source
files **natively** — it never shells out to the original linters at runtime — and
reports each one as a queryable record (tool, rule, reason, location). Consumers
are reviewers and CI jobs that want every new suppression, and its stated
justification, visible without reading the whole diff.

## Two standing goals on every task

The user drives product features and their request is the priority — but carry
two goals into *every* task. When either is the lowest-error path to what the
user asked, fold it into the same task without asking first; surface the rest as
follow-ups.

1. **Engineer the context for next time.** Realistic end-to-end tests that
   exercise what the user actually sees — especially when they report a bug
   existing tests missed — scripts that automate repetitive steps and shrink
   their output to signal, and terse `AGENTS.md` notes capturing what the code
   doesn't make obvious.
2. **Engineer the codebase and environment.** Keep the codebase clean and
   repeatable, and keep setup automated (`just bootstrap` from a clean clone).
   Strict gates plus local/CI parity on one pinned toolchain make results
   reproducible, not "works on my machine."

## Stack and composition

<!-- llmlint: ignore[agents_md_durable_and_terse] the create-repo baseline checker
     (`check_repo_baseline.py`) fails the repo unless this section records the shape,
     languages, references, and exclusions verbatim, so it is a required artifact
     rather than optional narrative. -->

- **Product shape:** cli
- **Language(s):** rust
- **References composed:** base.md, shapes/cli.md, languages/rust.md,
  intersections/rust-cli.md, ci.md, llmlint.md, releasing.md
- **Excluded, and why:** *asdf / direnv* — `rust-toolchain.toml` already pins the
  toolchain and rustup reads it, so a second pin would only drift. *Monorepo
  orchestration* — one crate, one deliverable. *Bench tier* — deferred, not
  dropped: speed is a product claim, so it lands once the parser set makes the
  numbers meaningful.
- **crates.io is dormant, not excluded:** `release-plz.toml` sets
  `publish = false` so versioning stays decoupled from the registry, and
  `release.yml`'s `publish-crate` job self-activates the day
  `CARGO_REGISTRY_TOKEN` is set. Until then the artifact ships via GitHub
  Releases, `install.sh`, and `cargo install --git`.

## The ignore-record contract (fixed)

`IgnoreDirective` and the report envelope are a **public, versioned contract**
consumed by downstream tooling. Do not change field names, the `scope` variants,
or the envelope shape unilaterally: bump `REPORT_VERSION` and update
`tests/golden/` in the same change. `tests/schema.rs` locks the serialized shape.
New fields must be optional, round-trip, and be omitted when empty.

The 1-based coordinates stay plain `u32` with public fields rather than newtypes:
the record is what every parser dispatch builds against, and the extractor's
cursor cannot emit a zero. The invariant that *is* enforceable at a trust
boundary — an envelope from a newer build — is rejected on deserialization.

## Adding a tool parser

One module under `src/tools/`, one line in `src/tools/mod.rs::registry()`, one
row in the README supported-tools table. Keep it to those three touch points so
parallel branches don't conflict. Parsers consume the extracted comments and
attributes from `src/comments.rs` — never re-scan raw lines, or string literals
will be misread as comments.

## Commits, releases, and merging

- **Squash-merge only, via PR, with auto-merge.** The default branch is
  protected: merge commits and rebase-merging are disabled, so one PR is one
  squash commit whose subject is the PR title. Queue with
  `gh pr merge --auto --squash`. Merged head branches auto-delete. Admins may
  bypass in a break-glass.
- **A new CI job is advisory until it is required.** Branch protection lists
  contexts by name, so adding a job means re-running the create-repo skill's
  `setup_github_governance.py` with the new context — otherwise a red run still
  merges.
- **The PR description becomes the squash commit body**, so it is history, not
  paperwork.
- **Merging a PR is the only human action in a release.** Never hand-edit a
  version, hand-tag, or hand-dispatch a publish; if a release needs that, the
  pipeline is broken.
- **`RELEASE_PLZ_TOKEN` must stay a PAT.** A tag pushed by the default
  `GITHUB_TOKEN` does not trigger other workflows, so `release.yml` would never
  build the binaries and the release would ship nothing — silently.
- **We are pre-1.0**, so a breaking change is a minor bump, not a major.
  Revisit the mapping in `release-plz.toml` at 1.0.

## Invariants (non-negotiable)

- The quality gate is strict: no warnings-only mode. A diagnostic is either an
  error or suppressed with a documented, tracked rationale. (We are a tool that
  surfaces suppressions — an unjustified `#[allow]` here is a self-own.)
- Validate all external / IO inputs at trust boundaries. An unreadable file or a
  malformed directive becomes a report `error` entry — never a panic.
- A routine step belongs in a `just` recipe, not a one-off command: CI runs the
  recipes, so anything outside them is unproven.
- Keep the artifact portable across Linux, macOS, and Windows.
- **Security is gate-level.** No secrets, credentials, or customer data in the
  tree; every grant least-privilege.

## Scripts and output are context

- Every script is quiet on success — a single line or nothing.
- On failure, print the exact error and a concrete suggested next action.
- Treat all command output as context the next agent has to read.

## Parity is the contract each parser owes

A parser is unproven until an e2e drives the **real** tool over a fixture and
shows it *fails* without the suppression and *passes* with it, while notignored
reports exactly that suppression. `tests/e2e/ruff_parity.rs` is the shape to
copy; the tool is pinned so the proof is reproducible. Coverage (95%, enforced)
is a floor that a mocked suite could also clear — parity is what makes the claim
true.

## Keeping the allowlist current

Grant `just` recipes, not raw `cargo`/`uv` wildcards: a wildcard on a build tool
is arbitrary code execution. When a routine command joins the workflow, add its
recipe to `.claude/settings.json` rather than re-approving it every session.

## Conventions

- Library-first: `src/lib.rs` holds the engine and is the public API; `src/main.rs`
  is a thin `clap` shell over `src/cli/`. Keep the CLI layer free of parsing logic
  so `--diff` and other modes slot in without touching parsers.
- Rule codes and reasons are captured **verbatim** as written in the source; do
  not normalize case, expand aliases, or re-order.
