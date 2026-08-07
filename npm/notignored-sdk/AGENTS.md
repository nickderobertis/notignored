# AGENTS.md — `notignored-sdk` (TypeScript)

Subtree rules. The repo-wide constraints are in the root `AGENTS.md`.

- **The public surface is `scan` plus the contract types and the error classes,
  and nothing else.** `src/binary.ts` and `src/contract.ts` are internals that
  `src/index.ts` does not re-export; keep it that way. Adding a verb means
  adding the CLI flag it mirrors, not a convenience the CLI cannot express.
- **The record types are the JSON's names** — `start_line`, not `startLine`. A
  camelCase mirror would be a second spelling of a published contract, and the
  first field added upstream would land in only one of them.
- **Validation at the boundary is strict and never lossy.** An unknown tool or
  scope, a missing field, or an envelope from a newer build is a
  `NotignoredContractError`. A suppression reporter that silently skipped a
  record it did not recognise would report an unjustified suppression as absent.
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
  branches the real binary cannot produce. Nx caches this project's `test`, and
  the crate's sources are not among its inputs — a parser change re-runs the
  crate's suite, not this one — so re-run `just nx run notignored-sdk-npm:test
  --skip-nx-cache` after touching `src/tools/`.
- **`node --test` is given an explicit glob** (`test/*.test.mjs`). Its default
  discovery treats every file under `test/` as a suite, which would run
  `support.mjs` and the fixture as tests.
