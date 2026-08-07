# AGENTS.md — `notignored-sdk` (Python)

Subtree rules. The repo-wide constraints are in the root `AGENTS.md`.

- **This is a scaffold, not an implementation.** It exists so the Nx graph and CI
  wiring are proven before the SDK lands. `tests/test_scaffold.py` is the
  placeholder tier: replace it as real surface arrives, never delete it to make a
  target pass.
- **Its lockfile is its own.** `uv.lock` here pins the dev tier (pytest); the
  repo root has no uv project. `bootstrap` runs `uv sync --locked`, so a
  dependency change means committing the refreshed lock in the same commit.
- **ruff comes from the repository's pin, not from this project.** `.ruff-version`
  at the root is the one source and `scripts/dev-tool.sh` resolves the binary
  `just bootstrap` installed. Only the rule selection lives in `pyproject.toml`.
- **No version of its own once it publishes.** `Cargo.toml` is the repo's single
  version source; wire this package to it the way the `notignored-cli` wheel is
  (`dynamic = ["version"]`) rather than adding a second one.
