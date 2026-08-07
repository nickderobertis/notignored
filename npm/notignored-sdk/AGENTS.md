# AGENTS.md — `notignored-sdk` (TypeScript)

Subtree rules. The repo-wide constraints are in the root `AGENTS.md`.

- **This is a scaffold, not an implementation.** It exists so the Nx graph and CI
  wiring are proven before the SDK lands. `test/scaffold.test.mjs` is the
  placeholder tier: replace it as real surface arrives, never delete it to make a
  target pass.
- **Do not confuse it with `npm/notignored/`.** That is the published CLI
  launcher, whose manifest and `PACKAGES` map `tests/packaging_contract.rs` locks
  against the release matrices. This directory publishes nothing yet and is not
  part of that contract.
- **`version` stays `0.0.0-managed`, and `private` stays true** until the SDK
  publishes. `Cargo.toml` is the repository's single version source (see the root
  `AGENTS.md`), so this manifest must never become a second one.
- **No dependencies, no lockfile of its own.** The suite is `node --test` and the
  linter is the repository's pinned biome, resolved by `scripts/dev-tool.sh` from
  the `tests/js-toolchain` pins. Adding a runtime dependency means adding a
  lockfile and wiring it into `bootstrap` in the same change.
- **`biome.json` carries no `$schema`** on purpose: the URL embeds a version, and
  `tests/js-toolchain/package.json` is the one place biome's version is pinned.
