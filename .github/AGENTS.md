# Workflow tooling

Private scheduling proofs parse the actual workflow graph with the published
Actions evaluator. A new runner expression or reusable call must resolve or fail
explicitly; never infer zero cost from an unknown route.

A same-repository release-plz branch plus a semantic version/changelog-only diff
permits a Mac omission. Author login cannot: release-plz uses the owner's PAT.
Unknown PR classification requires coverage. A draft (`draft: true`, nothing
else) defers the Mac lane: marking it ready starts a run of its own
(`ready_for_review`), which is where the lane is spent, so a draft-first pull
request still costs one PR run. Main pushes and scheduled smoke spend no Mac
runner; the explicit manual diagnostic defaults off.

Update the locked reusable fixture and its provenance when changing its
production call, including dynamic architecture resolution from screencomp.toml.
