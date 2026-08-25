# AGENTS.md — `notignored-sdk` (Python)

Subtree rules. The repo-wide constraints are in the root `AGENTS.md`.

- **The surface is fixed and small.** `scan` / `ascan`, the five record types, the
  two enums, and the error hierarchy — `tests/test_api.py` asserts `__all__` is
  exactly that set, and asserts both entry points' parameters against the
  approved `(paths, *, diff, diff_base, tools, cwd)` spelled out as a literal.
  **Which binary runs is not an argument.** The approved contract asks for both
  a fixed five-parameter signature and "an explicit path argument or env
  override"; those cannot both hold, and the resolved reading is that
  `NOTIGNORED_BIN` *is* the explicit override, with `PATH` as the fallback. Do
  not re-add a `binary=` parameter to close that gap — it was proposed, weighed,
  and settled. Anything else is an implementation detail behind a `_` module;
  adding to the surface is a deliberate decision, not a side effect.
- **`Tool` and `Scope` are real `enum.StrEnum`s**, which is why
  `requires-python` is `>=3.11`. A `(str, Enum)` lookalike compares equal to its
  wire value but stringifies as `Tool.RUFF` — a near-miss that reaches a log line
  or an f-string and is never noticed.
- **`scan` and `ascan` take the identical arguments.** They are one call in two
  forms, so a caller can swap either for the other; a flag added to one is a bug
  until it is on both, and `tests/test_api.py` compares the signatures.
- **Parsing is strict, except about keys.** An unknown tool, an unknown scope, an
  unknown `change`, a missing field, or a wrong type is a
  `NotignoredContractError` — never a dropped record, because a scan that quietly
  reports fewer suppressions than it found is worse than one that fails. Keys this
  SDK has never seen are carried past, because the record contract's own rule is
  that new fields are additive.
- **Every non-zero exit is `NotignoredExitError`, carrying the CLI's stderr.**
  Including the 2 the CLI uses for an unreadable file, which it names on stderr
  as well as in the report's `errors` — so nothing is lost, and a tree that could
  not be fully scanned can never be mistaken for a clean one. `Report.errors`
  stays on the record because the v1 envelope has it, but a returned report
  always has it empty.
- **The tests drive the binary this workspace builds.** `conftest.py` compiles
  it, which is why the `test` target names `crateSource` among its inputs —
  without it, a crate change would replay a cached green from before the contract
  moved. It names the crate's *sources*, not the crate *project*: that project's
  root is the repository root, so depending on it would make every file outside
  the SDK trees affect this one. **Nothing here stands in for the CLI**, and the
  dividing line is what the workspace binary can actually produce: every state it
  *can* reach — reports, `--diff`, the tool filter, a non-zero exit — is covered
  by driving that binary, and nothing else may stand in for those. The two states
  it *cannot* reach are covered without fabricating one: `tests/test_contract.py`
  calls the strict reader directly with the payload a broken or newer build would
  emit, and `tests/test_errors.py` points `NOTIGNORED_BIN` at real unrelated
  programs (`echo`, `true`) — the misconfiguration a user actually hits — to
  prove the plumbing carries that verdict out through `scan()`. A patched second
  build of the crate, or test-only behaviour in the CLI, is not the answer.
- **Its lockfile is its own.** `uv.lock` here pins the dev tier; the repo root has
  no uv project. `bootstrap` runs `uv sync --locked`, so a dependency change
  means committing the refreshed lock in the same commit.
- **ruff and mypy come from the repository's pins, not from this project.**
  `.ruff-version` / `.mypy-version` at the root are the one source and
  `scripts/dev-tool.sh` resolves the binaries `just bootstrap` installed. Only the
  rule selection lives in `pyproject.toml`.
- **`typecheck` covers `src` *and* `tests`**, through
  `scripts/python-sdk-typecheck.sh`. The suite is the SDK's executable
  specification, so a test calling `scan()` with the wrong argument types is a
  test asserting something the API does not promise. The pinned mypy lives in its
  own venv with nothing else in it, so the script points it at this project's
  interpreter with `--python-executable`; that is how it resolves pytest without
  adding a second mypy pin to the dev group.
- **`Cargo.toml` is still the only version source.** This package's `version` and
  its `notignored-cli` dependency are placeholders that
  `scripts/python-sdk-build.mjs` stamps at release time; `tests/test_packaging.py`
  builds that wheel on every gate run and reads the metadata back.
