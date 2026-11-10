# AGENTS.md — `pool-maintenance`

- **What it owns.** The budget of the pool maintenance `onevcs.toml` declares:
  `notignored-pool-maintenance-seconds`, how long one regular `just
  pool-maintenance` pass holds a warm slot out of service. The recipe owns the
  window and the target dirs; `measure.sh` only runs it.
- **Steady state, never backlog.** The figure is one pass over exactly one
  superseded build, and `measure.sh` refuses rather than times any other
  workload. Its header says how the tree gets there; the lists it leaves in
  `target/pool-maintenance/` are where to start when it refuses.
- **Its journeys live in `journeys/`**, a project of their own
  (`pool-maintenance-journeys`) because they drive the real pruner; see its
  `AGENTS.md`.
- **Release-prep only.** It builds the crate twice, so its target is
  `budgets-release`, which no `check`, `budgets` or affected run selects —
  `tests/e2e/nx_workspace.rs` asserts that from Nx's task graphs. CI runs
  `just budgets-release` after the release pull request's full sweep. Never
  rename it `budgets` or `budgets-host`, or every change that reaches this
  project pays for two builds.
- **Linux hosts.** onevcs pools run on POSIX hosts, so `measure.sh` uses GNU
  `touch -d` and `find`; it is not on the cross-platform legs.
