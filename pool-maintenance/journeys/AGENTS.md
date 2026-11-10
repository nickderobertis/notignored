# AGENTS.md — `pool-maintenance-journeys`

- **What it owns.** `test.sh`: the journeys of `../measure.sh` — the real
  script, the root justfile's `pool-maintenance` recipe and the pinned
  `cargo-prune-superseded`, over a `target/` a cargo double fills. They prove
  the staging and both refusals, never the figure, so no real build runs.
- **Its own project because it drives a host tool.** The pinned pruner is a real
  host tool, so these journeys stay out of `pool-maintenance`'s offline targets
  and name what they read (`measure.sh`, the root `justfile`) as inputs rather
  than depending on a project, as the e2e tier does.
- **Only cargo is a double.** A real gate build takes minutes and cannot be made
  to leave a cache-aged tree or a build that never settles. Keep the pruner,
  the recipe and `measure.sh` real; a journey over a stubbed pruner proves
  nothing about the workload it times.
