# AGENTS.md — `notignored-e2e` (the end-to-end journeys)

This directory is the crate's **e2e tier**: one test binary (`main.rs`) that
drives the compiled `notignored`, the pinned linters it claims parity with,
loopback GitHub servers, real `git`, and the packaging builds. It is its own Nx
project so a change that reaches neither it nor the crate does not pay for it;
it depends on `notignored`, so anything the crate owns selects it.

- **Its bootstrap is the pinned toolchain.** `bootstrap` installs ruff, mypy,
  pyright, ty, eslint, biome, tsc, shellcheck and llmlint after the crate's own
  toolchain. A journey that needs a new tool adds its installer there.
- **Inputs are everything the journeys read.** `^default` is the whole crate
  project (sources, scripts, workflows, manifests), and `e2eSharedTests` names
  what it reads from the integration project's tree (`fixtures/`, `golden/`,
  `support/`). A cached green from before one of those moved would prove
  nothing.
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
  variable of the enclosing task cleared.
