# AGENTS.md — `notignored-sdk` (TypeScript)

Subtree rules. The repo-wide constraints are in the root `AGENTS.md`.

- **The public surface is `scan` plus the contract types and the error classes,
  and nothing else** — `test/surface.test.mjs` reads the emitted `.d.ts` and
  fails on any addition. `src/binary.ts` and `src/contract.ts` are internals
  `src/index.ts` does not re-export, and `ScanOptions` is a parameter type, not
  a product: exporting it would publish a name whose next added option is a
  breaking change to something nobody meant to promise. Adding a verb means
  adding the CLI flag it mirrors, not a convenience the CLI cannot express.
- **`scan` takes no binary argument.** `NOTIGNORED_BIN` is the whole of explicit
  selection, then the `notignored-cli` launcher, then `PATH`. One mechanism,
  settable by a CI step or a harness without touching a call site.
- **Only a completed run resolves.** Any non-zero exit is a
  `NotignoredExitError` carrying `exitCode`, `signal`, and verbatim `stderr` —
  including the unreadable-file case, where the binary prints the envelope *and*
  exits 2. Resolving that stdout would let a caller read "this tree is clean"
  off a scan that never opened one of its files.
- **The record types are the JSON's names** — `start_line`, not `startLine`. A
  camelCase mirror would be a second spelling of a published contract, and the
  first field added upstream would land in only one of them.
- **Validation at the boundary is strict in both directions.** An unknown tool
  or scope, a missing field, an envelope from a newer build — and a field this
  version does not define, at any of the four objects (`Report`,
  `IgnoreDirective`, `Suppressed`, `ReportError`) — is a
  `NotignoredContractError`. A suppression reporter that silently skipped a
  record it did not recognise would report an unjustified suppression as absent.
  The price is real and deliberate: **adding a field to the crate's record means
  adding it to the field lists in `src/contract.ts` in the same change**, or
  this reader refuses the new build's reports even within version 1.
- **Do not confuse this with `npm/notignored/`.** That is the CLI launcher whose
  `PACKAGES` map `tests/packaging_contract.rs` locks against the release
  matrices. This publishes `notignored-sdk`, from `scripts/pack.mjs`.
- **`version` stays `0.0.0-managed` in the committed manifest.** `Cargo.toml` is
  the repository's single version source; `scripts/pack.mjs` stamps it into the
  copy that publishes, and `test/packaged.test.mjs` fails if the two ever agree.
- **No dependencies and no lockfile of its own.** The suite is `node --test`,
  biome and tsc come from the repository's one JS pin
  (`tests/js-toolchain/package.json`, installed to `.dev/js` by
  `scripts/setup-js.sh`), and `tsconfig.json` points `typeRoots` there for the
  Node type definitions. `notignored-cli` is resolved at runtime *if a consumer
  installed it* — it is deliberately not a dependency, because `PATH` and
  `NOTIGNORED_BIN` are equally supported ways to have the binary.
- **`biome.json` carries no `$schema`** on purpose: the URL embeds a version, and
  `tests/js-toolchain/package.json` is the one place biome's version is pinned.
  The `lint` target passes `--error-on-warnings`, because the repo's gate has no
  warnings-only mode and biome exits 0 on a warning otherwise.
- **The suite drives the real binary; nothing is mocked.** `test/support.mjs`
  finds `target/{debug,release}/notignored` and builds it if it is not there.
  `test/fixtures/not-notignored.mjs` is not a stand-in for it: it is a program
  that deliberately *is not* `notignored`, for the resolution and contract
  branches the real binary cannot produce. Nx caches this project's `test`, so
  its inputs name `crateSource` (`src/`, `Cargo.toml`, `Cargo.lock`) alongside
  `default` — the same input the Python SDK names, for the same reason: a cached
  green from before a parser moved would prove nothing.
- **`node --test` is given an explicit glob** (`test/*.test.mjs`). Its default
  discovery treats every file under `test/` as a suite, which would run
  `support.mjs` and the fixture as tests.
