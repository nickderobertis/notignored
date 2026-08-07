# AGENTS.md — `notignored-sdk` (Python)

Subtree rules. The repo-wide constraints are in the root `AGENTS.md`.

- **The surface is fixed and small.** `scan` / `ascan`, the five record types, the
  two enums, and the error hierarchy — `tests/test_api.py` asserts `__all__` is
  exactly that set. Anything else is an implementation detail behind a `_`
  module; adding to the surface is a deliberate decision, not a side effect.
- **`scan` and `ascan` take the identical arguments.** They are one call in two
  forms, so a caller can swap either for the other; a flag added to one is a bug
  until it is on both, and `tests/test_api.py` compares the signatures.
- **Parsing is strict, except about keys.** An unknown tool, an unknown scope, a
  missing field, or a wrong type is a `NotignoredContractError` — never a
  dropped record, because a scan that quietly reports fewer suppressions than it
  found is worse than one that fails. Keys this SDK has never seen are carried
  past, because the record contract's own rule is that new fields are additive.
- **A non-zero exit is not automatically an error.** The CLI exits 2 for an
  unreadable file *and still prints the report that names it*. The report wins
  whenever there is one; `NotignoredExitError` is for a run that produced none.
- **The tests drive the binary this workspace builds.** `conftest.py` compiles
  it, which is why the `test` target names `crateSource` among its inputs —
  without it, a crate change would replay a cached green from before the contract
  moved. It names the crate's *sources*, not the crate *project*: that project's
  root is the repository root, so depending on it would make every file outside
  the SDK trees affect this one. `stub_binary` is not a mock of the CLI: it stands
  in for a *broken or newer* one, which is the only way the contract-error
  branches can be reached at all.
- **Its lockfile is its own.** `uv.lock` here pins the dev tier; the repo root has
  no uv project. `bootstrap` runs `uv sync --locked`, so a dependency change
  means committing the refreshed lock in the same commit.
- **ruff and mypy come from the repository's pins, not from this project.**
  `.ruff-version` / `.mypy-version` at the root are the one source and
  `scripts/dev-tool.sh` resolves the binaries `just bootstrap` installed. Only the
  rule selection lives in `pyproject.toml`.
- **`typecheck` covers `src`, not `tests`.** The pinned mypy lives in its own venv
  with nothing else in it, so it cannot see pytest; type-checking the suite there
  would report missing stubs, not real errors. The suite is proven by running.
- **`Cargo.toml` is still the only version source.** This package's `version` and
  its `notignored-cli` dependency are placeholders that
  `scripts/python-sdk-build.mjs` stamps at release time; `tests/test_packaging.py`
  builds that wheel on every gate run and reads the metadata back.
