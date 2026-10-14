# AGENTS.md — `pool-maintenance`

- **What it owns.** The budget of the pool maintenance `onevcs.toml` declares:
  `notignored-pool-maintenance-seconds`, how long one regular `just
  pool-maintenance` pass holds a warm slot out of service. The recipe owns the
  window and the target dirs; `measure.sh` only runs it.
- **Steady state, never backlog.** The figure is the pass after one run's worth
  of superseded builds. `measure.sh` clears the backlog untimed, builds the
  gate's test binaries under a debuginfo profile (new metadata hashes in the
  same profile dir), backdates exactly what that build created, rebuilds
  ordinarily, then times one pass. It fails rather than reports when the pass
  removed nothing or removed an ordinary build; the lists stay in
  `target/pool-maintenance/`.
- **Release-prep only.** It builds the crate twice, so its target is
  `budgets-release`, which no `check`, `budgets` or affected run selects —
  `tests/e2e/nx_workspace.rs` asserts that from Nx's task graphs. CI runs
  `just budgets-release` after the release pull request's full sweep. Never
  rename it `budgets` or `budgets-host`, or every change that reaches this
  project pays for two builds.
- **Linux hosts.** onevcs pools run on POSIX hosts, so `measure.sh` uses GNU
  `touch -d` and `find`; it is not on the cross-platform legs.
