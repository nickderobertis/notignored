# AGENTS.md — `notignored-integration` (the contract suites)

This directory is the crate's **integration tier**: the `tests/*.rs` suites that
hold files to each other — workflows, the action, the installer, the packaging
manifests, the report schema — by reading them. `tests/e2e/` below it is a
project of its own (`notignored-e2e`); Nx maps a file to the project whose root
is its longest prefix, so nothing here selects the journeys.

- **One cargo package, three Nx projects.** The tiers are split in the graph,
  not into crates: the journeys run the binary through `assert_cmd`'s
  `cargo_bin`, which only resolves inside the package that builds it, and the
  `[package] include` set release-plz reads must stay whole. So this project's
  `test` selects its binaries with a nextest filter
  (`kind(test) and not binary(e2e)`), not with `-p`, and a new suite needs no
  graph edit.
- **Its coverage counts towards the crate's floor, but it never reports.** `test`
  records its profiles under `target/coverage-profiles/notignored-integration`
  (`scripts/coverage.sh tier`), and `notignored:coverage` combines every tier's.
- **`fixtures/`, `golden/` and `support/` live here but serve the journeys too.**
  The e2e project names them as inputs (`e2eSharedTests` in `nx.json`), so a
  change to one re-runs both tiers. A new directory the journeys read belongs in
  that list in the same change.
- **`format` is the crate's.** `cargo fmt` formats the package as one unit, so
  this project's `format` waits on `notignored:format` rather than running a
  second formatter over the same files at once.
