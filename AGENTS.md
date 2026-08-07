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
- **Language(s):** rust, python, typescript
- **References composed:** base.md, shapes/cli.md, languages/rust.md,
  intersections/rust-cli.md, ci.md, llmlint.md, releasing.md, monorepo.md
- **Monorepo, since the SDKs:** three deliverables in three languages — the Rust
  CLI, `python/notignored-sdk`, `npm/notignored-sdk` — is past the threshold where
  monorepo.md applies, so it is composed rather than excluded. It was excluded
  while there was one crate and one deliverable; that is no longer true.
- **Excluded, and why:** *asdf / direnv* — `rust-toolchain.toml` already pins the
  toolchain and rustup reads it, so a second pin would only drift. *Bench tier* —
  deferred, not dropped: speed is a product claim, so it lands once the parser set
  makes the numbers meaningful.
- **crates.io is dormant, not excluded:** `release-plz.toml` sets
  `publish = false` so versioning stays decoupled from the registry, and
  `release.yml`'s `publish-crate` job self-activates the day
  `CARGO_REGISTRY_TOKEN` is set. Until then the artifact ships via GitHub
  Releases, `install.sh`, `cargo install --git`, and the PyPI/npm packages below.

## The project graph

Three projects, one graph: `notignored` (the crate, rooted at the repo root),
`notignored-sdk-python` and `notignored-sdk-npm` — each shipped, and each with
its own nested `AGENTS.md` and a `.github/CODEOWNERS` line routing its reviews.
They publish `notignored-sdk` to PyPI and to npm respectively; see "The registry
packages".

Nx **runs** targets; it never decides what one does. A target names its project's
own language-native tool (`_crate-*` recipes for cargo, ruff/mypy for the Python
SDK, biome for the TS SDK), and the repo-wide verbs — `bootstrap`, `check`,
`lint`, `test`, `format`, `fmt-check`, `upgrade` — shell out to
`nx run-many`/`nx affected` rather than looping over projects. Adding a project
means a `project.json` declaring the same six target names (`bootstrap`,
`format`, `format-check`, `lint`, `test`, `check`) plus one `scope:` tag, a
nested `AGENTS.md`, a `CODEOWNERS` line, and a line in
`tests/e2e/nx_workspace.rs`; nothing in the justfile has to change. A seventh
target is a project's own business — the Python SDK's `typecheck` is one, folded
into its `check` rather than into a repo-wide verb, because only it has one.

Affected selection rests on **project roots, not `namedInputs`** — Nx maps a
changed file to the project whose root is its longest prefix, and treats a
`{workspaceRoot}` glob in a project's inputs as touching that whole project. The
crate's root *is* the repo root, so a `{workspaceRoot}/**/*` input would make it
affected by every SDK-only change; `nx.json` therefore keeps `default` for it and
names shared pins per project (`pythonToolchain`, `jsToolchain`). The one
deliberate cross-project input is `crateSource` — `src/`, `Cargo.toml`,
`Cargo.lock` — which **both** SDKs' `test` targets name, because each suite
compiles and drives that binary, and a cached green from before a contract moved
would prove nothing. It names the crate's *sources* rather than depending on the
crate *project*, which would drag in the whole repo root.
`tests/e2e/nx_workspace.rs` locks that mapping, because CI skips the `cross`,
`msrv`, `deny`, and `install` matrices on `just affected-crate` and a file that
stopped mapping to the crate would turn a skipped matrix into an unproven
artifact. Everything downstream of that fails **closed**: with no derivable merge
base, `scripts/nx-affected.sh` runs the whole graph and says so.

`just bootstrap` is serialized (`--parallel=1`): projects share the
`scripts/setup-*.sh` installers, which recreate a `.dev/<tool>` tree from scratch
when a pin moves, and two of those at once race on the same directory.

**Boundaries are enforced on tags, over the whole graph.** Each project declares
one `scope:` tag and the CLI is the base layer — an SDK may depend on it, nothing
may depend on an SDK. `tests/e2e/nx_workspace.rs` checks every edge Nx resolved
against that layering and that the graph stays acyclic. Not Nx's
`@nx/enforce-module-boundaries`: that rule is ESLint's and reaches only the one
TypeScript project, so it could not see an edge from the Rust crate at all.

**`scripts/nx.sh` is quiet on success and preserves what it swallowed.** Every
gate recipe runs through it, so its output is `just check`'s output. A green run
is one line naming `.logs/nx.log`; a failure streams that log to stderr and names
it again. `scripts/preserved-log.sh` owns the file: owner-only, truncated per
*invocation*, and diverted to `nx.<pid>.log` when an enclosing run has claimed the
stable path — which happens on every gate, because `just check` runs the e2e suite
through Nx and those journeys spawn the wrapper again. Credential-shaped
environment values are masked on the way in, since a log outlives the terminal.
`NOTIGNORED_NX_SHOW_OUTPUT=1` streams instead, for the callers that parse Nx's
stdout.

## The ignore-record contract (fixed)

`IgnoreDirective` and the report envelope are a **public, versioned contract**
consumed by downstream tooling. Do not change field names, the `scope` variants,
or the envelope shape unilaterally: bump `REPORT_VERSION` and update
`tests/golden/` in the same change. `tests/schema.rs` locks the serialized shape.
New fields must be optional, round-trip, and be omitted when empty.

A **third** place moves with them: `npm/notignored-sdk/src/contract.ts` refuses
a field it does not know, at every object in the envelope, rather than drop one
whose presence changes what a record means. That is the SDK's decision and its
`AGENTS.md` records the trade — but it means a field added here and not there
makes the TypeScript SDK reject this build's reports.

The 1-based coordinates stay plain `u32` with public fields rather than newtypes:
the record is what every parser dispatch builds against, and the extractor's
cursor cannot emit a zero. The invariant that *is* enforceable at a trust
boundary — an envelope from a newer build — is rejected on deserialization.

## Adding a tool parser

One module under `src/tools/`, one line in `src/tools/mod.rs::registry()`, one
row in the README supported-tools table, and one directive in
`tests/fixtures/polyglot/` (re-bless with `just bless`). Keep it to those four
touch points so parallel branches don't conflict. The fixture is not optional:
`tests/e2e/polyglot.rs` fails when a registered tool is missing from that tree,
and it is the only place a parser that quietly stopped applying — a narrowed
language claim, a registry line lost in a merge — shows up, because every other
suite asks its own parser directly. Parsers consume the comments,
attributes, and item punctuation `src/comments.rs` extracted — never re-scan raw
lines, or string literals will be misread as comments. Grammar shared by a
*family* of tools lives in a private sibling module (`src/tools/python.rs` serves
mypy/pyright/ty); it is not a tool and stays out of the registry.

`ToolParser` is **fixed at three methods** returning directives and nothing else;
`tests/tools_contract.rs` locks the signatures. A syntax that can be malformed in
a way the record cannot express (an unclosed llmlint block) keeps that richer
result as an *inherent* method on its own parser — `LlmlintParser::scan` — which
`scan_files` folds into the report. Do not widen the trait for one tool's defect.

A tool's scope is what it honours, not what its docs headline — ty reads an
own-line directive as covering the line below. Derive each from the real tool,
but report only the scopes the record contract specifies: a checker may honour
more than we claim, and widening that unasked changes the contract.

One line can carry several tools' directives. Each record's `raw` and `reason`
must stop at the next one, or a live suppression is filed as its neighbour's
justification. `src/tools/python.rs::segments` owns that boundary; a new Python
parser adds an `opens_directive` recognizer to it.

## The GitHub Action

`action.yml` (composite, repo root) + `scripts/action/comment.sh` are the
product's review surface. Keep the judgment in Rust: `--format markdown` renders
the whole comment body, golden-tested over the fixture counts
(`tests/golden/markdown/`), so the composite's shell only moves bytes. The
`--max-entries` cap needs more findings than those fixtures hold, so it is proven
either side of its boundary in the renderer's own unit tests instead. Its two
scripts are proven by *lifting them out of `action.yml`* and running them
(`tests/e2e/action_scan.rs`, `tests/e2e/action_comment.rs`); a copy in a test
would keep passing after the action stopped doing what it says. Nothing is
mocked: github.com is the one host those journeys cannot own, so `gh` talks HTTP
to a real server they run on loopback, and everything else is a real repository,
the real binary, and the real `gh`.

Never interpolate `${{ github.event.* }}` (or an input) into a `run:` script: on
a fork's pull request that text is attacker-controlled and `${{ }}` splices it in
as shell source. Pass it through `env`, or read the payload as data from
`$GITHUB_EVENT_PATH`. `tests/action_contract.rs` fails the build otherwise, and
also gates the inputs, outputs, and the sticky marker the renderer and the
comment script must agree on.

`.github/workflows/notignored.yml` dogfoods it with `version: local`, which
builds the branch's own source. It is **not a required check** and must not
become one: it is skipped on fork pull requests, whose read-only token cannot
upsert a comment, and a required context that never reports would block them
forever.

## The screenshots, and the visual gate

The product is a rendered report, so the README leads with one: an animated hero
GIF and a gallery of SVGs, all **real output from the real release binary** over
the committed fixture in `screenshots/fixture/`. `screenshots/AGENTS.md` owns the
detail — the scenes, why the bytes are reproducible without a container, and how
to bless a change. Three things belong here because they cross the repo:

- **`--color` is what makes the gallery possible.** `render_human_colored` takes a
  plain `bool`; the TTY / `NO_COLOR` / `TERM=dumb` decision is resolved once in
  `Cli::color_enabled`, at the I/O boundary, and `--color always` overrides all of
  it so a capture can force colour through a pipe. `json` and `markdown` are
  contracts, not presentation, and `tests/e2e/color.rs` proves them byte-identical
  whatever the flag says — that is what keeps every golden report out of the blast
  radius of a formatting change.
- **Screenshots are informational and stay out of `just check`.** Capturing needs
  `freeze`, a release build, and a network fetch on a cold machine; the gate stays
  offline and toolchain-only. `tests/screenshots_contract.rs` is the part that
  *is* in the gate: it reads the config as text and fails the build when the
  `freeze` pin, the capture container, `[guard].paths`, the committed baseline,
  and the README embeds stop agreeing — none of which the capture itself would
  catch, because a drifted pin only shows up as a red CI nobody can explain.
- **`Visual docs` is deliberately not a required check**, for the same reason
  `notignored.yml` is not. screencomp's `report` lane needs `contents: write` to
  push the gh-pages gallery and `pull-requests: write` to upsert its sticky
  comment, and a fork's pull request is granted neither — a required context that
  can never report green would block outside contributors forever. On a branch of
  this repository it still runs with `fail-on-drift: true` and turns the pull
  request red, which is the review moment a formatting change deserves; read a red
  `Visual docs` as "the output moved — `just screenshots-bless` or explain it",
  not as advisory noise. To reverse this, register
  `visual-docs / report (x86_64)` by re-running the create-repo skill's
  `setup_github_governance.py` with that context (see the required-checks note
  below) — changing the workflow file alone does nothing.

## The registry packages

`pip install notignored-cli` and `npm install -g notignored-cli` ship the same
prebuilt binary the Release attaches — nothing compiles at install time.
`pyproject.toml` (maturin `bindings = "bin"`) is the whole wheel; `npm/notignored/`
is the committed launcher and `scripts/npm-build.mjs` generates the five
`notignored-cli-<platform>-<arch>` packages its `optionalDependencies` resolve to.
Those platform names stay **unscoped**: a `@scope/` name needs an npm org, which a
publish token cannot create.

The two SDKs are the fourth and fifth packages, and the only ones that are not
the binary. `pip install notignored-sdk` is a pure-Python client that *drives*
it, so it depends on `notignored-cli==<the same version>` and one `py3-none-any`
wheel serves every platform; its gate is the proof — `tests/test_packaging.py`
runs `scripts/python-sdk-build.mjs` and `uv build` on every gate run and reads
the version and the pin back out of the wheel's metadata, so the release job only
has to run the same two commands. `npm install notignored-sdk` compiles to
JavaScript and *resolves* whichever `notignored` its consumer installed, so
`notignored-cli` is deliberately not a dependency of it; it rides the
`NPM_PUBLISH` / `NPM_TOKEN` gating through `release.yml`'s `build-sdk-npm` job
and the existing `publish-npm` / `verify-npm` pair. Keep each SDK's public
surface, its strict parsing, and its forms as its nested `AGENTS.md` describes
them.

**Cargo.toml is the only version source.** The wheel takes it via `dynamic =
["version"]`, the npm packages via `npm-build.mjs`, the Python SDK via
`python-sdk-build.mjs`, the TypeScript SDK via
`npm/notignored-sdk/scripts/pack.mjs`; the committed npm manifests carry
`0.0.0-managed` and the committed Python SDK manifest `0.0.0.dev0` with an
*unpinned* `notignored-cli`, so none can become a second one — release-plz bumps
`Cargo.toml` alone. Adding a target means the release matrices, `npm-build.mjs`,
and the launcher's `PACKAGES` map together; `tests/packaging_contract.rs` fails
the build when they disagree, and `tests/e2e/packaging.rs` builds and installs
both packages from the real binary on every gate run. A test never spells a
version out either — it reads `support::cargo_version()`, because a release PR's
only payload *is* the bump, so a literal turns the one commit class that must
always be green into a red gate.

Publishing is token-based (`PYPI_TOKEN` / `NPM_TOKEN`), switched by the
`PYPI_PUBLISH` / `NPM_PUBLISH` repository variables — not Trusted Publishing, so
never add an `environment:` or OIDC claim expecting one. The *build* jobs stay
ungated on purpose: a packaging break must redden a release even while publishing
is off.

What the registries actually serve is proven by installing from them, per
**platform-and-architecture** — that pair is what selects a wheel and a platform
package, so an OS-keyed matrix would leave one of the five artifacts published and
never installed. `release.yml`'s `verify-pypi`/`verify-npm` install the
just-published version on `ubuntu-latest`, `macos-latest` (arm64),
`macos-15-intel`, and `windows-latest`, and
`.github/workflows/published-smoke.yml` sweeps `latest` the same way weekly so rot
between releases surfaces without one. Both assert through **one**
`scripts/smoke-published.sh` over `tests/fixtures/smoke/` and
`tests/golden/smoke.json`; `tests/e2e/smoke.rs` runs that same file over the build
under test, which is what keeps a workflow's expectations from drifting from the
parser that ships (re-bless with `just bless`). Keep the sweep toolchain-free — a
weekly job that compiled the crate is the first one switched off. `linux-arm64` is
the one artifact with no verify leg, because `setup-python` on `ubuntu-24.04-arm`
is not something a release is the place to discover; it rests on the build
matrices and `tests/e2e/packaging.rs`. `macos-15-intel` is hosted through August
2027 — when it retires, replace the label rather than dropping the leg.

**The GitHub Release is the third install surface, and it is verified the same
way.** `taiki-e/upload-rust-binary-action` derives *both* published names from the
one `archive:` stem by giving it an extension — `<stem>.tar.gz` and
`<stem>.sha256`, siblings. `scripts/install.sh` treated the checksum as a suffix
on the archive name instead, so it 404'd its own checksum and refused to install
on every release through v0.1.11 — the default path of `uses: …/notignored@v0`,
dead for every consumer. Nothing saw it: `tests/e2e/installer.rs` served a fixture
named after the script rather than after a release, and `notignored.yml` dogfoods
with `version: local`, which compiles from source and downloads nothing. Both
markers in `install.sh` are now rendered per target and held against a real
release's asset listing (`tests/fixtures/release-assets/`), which no template here
can talk into agreeing with it, and `release.yml`'s `verify-install` plus the
sweep's `github-release` job install from the live Release. Three runners, not
four: the ZIP branch needs `unzip` under a POSIX shell, which the Windows image
lacks, so Windows keeps `cargo install`.

**The install is the propagation probe, and the only one.** A publish is not one
event: PyPI's JSON API answers while `pip install` still resolves through a simple
index that converges later and per CDN edge, so a wait step polling that API said
"available" and the install then failed — v0.1.4, v0.1.5 and v0.1.6 each went red
having published fine. Every pinned-version install therefore runs through
`scripts/retry-install.sh` (bounded, ~10 minutes, last error shown) and carries
its client's cache-bypass flag, or the retry re-reads the "no such version" page
it just cached. Only the install retries — the smoke assertion after it stays
single-shot, so a wrong version fails now instead of in ten minutes.

## Commits, releases, and merging

- **Squash-merge only, via PR, with auto-merge.** The default branch is
  protected: merge commits and rebase-merging are disabled, so one PR is one
  squash commit whose subject is the PR title. Queue with
  `gh pr merge --auto --squash`. Merged head branches auto-delete. Admins may
  bypass in a break-glass.
- **A new CI job is advisory until it is required.** Branch protection lists
  contexts by name, so adding a job means re-running the create-repo skill's
  `setup_github_governance.py` with the new context — otherwise a red run still
  merges. Release-triggered and scheduled jobs (`release.yml`,
  `published-smoke.yml`) are the exception and must stay unrequired: they report
  no pull-request context, so requiring one would block every PR forever — the
  same trap `notignored.yml` and `visual-docs.yml` are kept out of (each for the
  fork-pull-request reason recorded with it). `ci.yml`'s `changes` job is the
  third exception and stays unrequired: it exists only to answer whether the Rust
  matrices run, it fails closed to "run them", and requiring it would add a
  context that says nothing about the code. The names it gates — `gate`, `cross`,
  `msrv`, `deny`, `install` — are unchanged, and a job skipped by an `if:`
  satisfies its required context.
- **The PR description becomes the squash commit body**, so it is history, not
  paperwork.
- **Merging a PR is the only human action in a release.** Never hand-edit a
  version, hand-tag, or hand-dispatch a publish; if a release needs that, the
  pipeline is broken.
- **`RELEASE_PLZ_TOKEN` must stay a PAT.** A tag pushed by the default
  `GITHUB_TOKEN` does not trigger other workflows, so `release.yml` would never
  build the binaries and the release would ship nothing — silently.
- **`@v0` is the action's consumption ref, and the release maintains it.**
  release-plz cuts only exact `vX.Y.Z`, so `release.yml`'s `major-tag` job
  force-moves a floating major tag — derived from the release tag, so `v1` starts
  moving at 1.0 with no edit — and it runs *last*, gated on every publish and
  verify job, because `@v0` must never resolve to a release whose artifacts
  failed to publish. `scripts/update-major-tag.sh` refuses a pre-release and
  refuses to walk the tag backwards onto an older release. README examples use
  `@v0`; the dogfood workflow stays `uses: ./`.
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
copy; every tool is pinned — a `.<tool>-version` file plus the `scripts/setup-*`
installer `just bootstrap` runs, or `rust-toolchain.toml` for clippy — so the
proof is reproducible. Coverage (95%, enforced) is a floor that a mocked suite
could also clear — parity is what makes the claim true.

One installer per tool family, each owning its own pins: renaming or folding one
into another collides with whatever branch owns the other family. A new tool
joins the family whose installer already speaks its packaging.

A tool with no pass/fail verdict to compare against still owes agreement on the
directive set: llmlint's parity runs `llmlint check-ignores`, its deterministic
model-free validator, and asserts the same files, lines, and rules. **Never
reach for llmlint's judge tier from a test** — it is a paid model call, so a
suite that used it would be neither free nor deterministic.

Never compare a path a checker reported against an expected one directly — send
it through `support::relative_to`. Tools disagree about absolute vs relative, and
on Windows two spellings of the same path (`d:/a/…` vs the verbatim `\\?\D:\a\…`
`canonicalize` returns) match on neither case, separator, nor prefix. The Linux
gate cannot see any of that; `support::paths` proves it with hand-written paths.

A fixture holding the *reason-less* form of a directive earns its keep with an
`llmlint: ignore-file[suppressions_justified]` footer — adding a reason instead
would delete the only coverage of that form. Put it after the code so the line
numbers the assertions cite do not move. That footer is itself a directive we
parse, so a suite asserting on a fixture's *whole* record set scopes its run with
`--tool`; only the golden reports scan unfiltered.

For a family of forms, `tests/e2e/python_types_parity.rs` scales it: every
fixture is the *same* program with a directive in a different slot, one test
asserts they differ only in comments, and one checker run per tool decides the
whole family. That keeps a slow checker (pyright) to a single invocation and
makes `violation.py` a control rather than a separate program.

The JS family is the one installer that does not read a `.<tool>-version` file:
`scripts/setup-js.sh` takes its pins from `tests/js-toolchain/package.json`
(eslint/biome/tsc, needs Node).

Where a tool parses its own reason — eslint's ` -- ` description, in
`suppressedMessages[].suppressions[].justification` — the e2e asserts our
extraction against *its* reading, not against a literal.

Not every parity proof can be a pass/fail flip. A mismatched or unclosed
`biome-ignore-start` is only a *warning* in biome, which still exits 0 and still
honours the range to end-of-file, so exit status cannot discriminate it. Those
journeys assert on biome's own diagnostic text via `--reporter=json`
(`support::biome_diagnostics`) instead. Reach for that shape only where the tool
genuinely has no failing exit to offer.

## Keeping the allowlist current

Grant `just` recipes, not raw `cargo`/`uv` wildcards, and grant the **exact**
command — `allow` carries no `:*` at all. A wildcard on a build tool is
arbitrary code execution, `just run *ARGS` interpolates its arguments into the
shell, and a trailing wildcard on `git commit`/`switch`/`branch` is an unbounded
write; those prompt, once each, which is the right friction on a write. `deny`
blocks outright what the sections above already forbid: force-push,
hand-tagging, `cargo publish`, `gh auth`/`secret`/`variable`. Add a routine
command's exact form here rather than re-approving it every session.

## Conventions

- Library-first: `src/lib.rs` holds the engine and is the public API; `src/main.rs`
  is a thin `clap` shell over `src/cli/`. Keep the CLI layer free of parsing logic
  so `--diff` and other modes slot in without touching parsers.
- Rule codes and reasons are captured **verbatim** as written in the source; do
  not normalize case, expand aliases, or re-order.
- `--diff` (`src/diff.rs`) shells out to real `git`: the no-shell-out rule is
  about the linters whose directives we parse, not infrastructure. Its semantics
  mirror llmlint's exactly, so a project running both can predict either — keep
  them in step.
