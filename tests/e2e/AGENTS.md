# AGENTS.md — `notignored-e2e` (the end-to-end journeys)

This directory is the crate's **e2e tier**: one test binary (`main.rs`) that
drives the compiled `notignored`, the pinned linters it claims parity with,
loopback GitHub servers, real `git`, and the packaging builds. It is the
expensive tier, so it sits behind its own edge.

- **It depends on no project; it names what it reads.** Like the SDKs, its
  `test` names the crate's sources and the root files the journeys drive —
  `e2eRootInputs` in `nx.json` — rather than depending on the crate project,
  whose root is the whole repository: a change to `AGENTS.md`, `docs/` or
  `deny.toml` does not run it. Each glob there is also what selects it, so **a
  journey that starts reading a root file outside that list adds it there in the
  same change**, or a cached green from before that file moved is replayed.
  `e2eSharedTests` names what it reads from the integration project's tree.
- **The binary it drives is built by its own run.** The journeys are an
  integration-test target of the crate's package, so cargo builds the
  `notignored` binary for them; there is no separate `build` task to wait on.
- **Its bootstrap installs the pinned parity toolchain** after the crate's own.
  A journey that needs a new tool adds its installer to `_crate-e2e-bootstrap`.
- **Its coverage counts towards the crate's floor, but it never reports.** `test`
  records its profiles under `target/coverage-profiles/notignored-e2e`;
  `notignored:coverage` combines them with the other tiers' and enforces 95% —
  most of the binary's lines are covered only from here.
- **The packaging journeys stay in this project, deliberately.** They are the
  slowest (`verify_npm`, `packaging`, `installer`, …), but the whole tier runs in
  about 30 seconds of wall clock on a warm build, they share this binary's
  `support` module, and a separate project needs its own directory root —
  moving nine journeys whose paths AGENTS.md, the workflows and the scripts
  cite. Split them out when the tier's measured time makes it worth that.
- **Graph journeys run Nx read-only, or in a scratch copy.** `nx_workspace.rs`
  runs inside an Nx task; a nested command that wrote this workspace's cache
  would race its parent. The `just affected-crate` journey makes its commits in
  a scratch repository holding the graph's real files, with every `NX_*`
  variable of the enclosing task cleared; `coverage_tiers.rs` likewise clears
  the enclosing instrumented run's variables before driving its own.
