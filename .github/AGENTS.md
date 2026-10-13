# Workflow tooling

Private scheduling proofs parse the actual workflow graph with the published
Actions evaluator. A new runner expression or reusable call must resolve or fail
explicitly; never infer zero cost from an unknown route.

A same-repository release-plz branch plus a semantic version/changelog-only diff
permits a Mac omission. Author login cannot: release-plz uses the owner's PAT.
Unknown PR classification requires coverage. Main pushes and scheduled smoke
spend no Mac runner; the explicit manual diagnostic defaults off.

The baseline workflows were parsed from b107580. The reusable screencomp workflow
is parsed from the v0.4.2 source at the commit and source/semantic hashes in
`fixtures/reusable/lock.json`; update its locked
fixture when changing the production call, including its dynamic arch output
resolution from screencomp.toml.
