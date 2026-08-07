"""What every journey here drives: the real binary this workspace builds.

Nothing in this suite mocks or stands in for the CLI. `notignored_binary` is
compiled from the crate beside this project — so a report shape that moved in
`src/` fails here on the same commit that moved it — and an autouse fixture puts
it on `NOTIGNORED_BIN`, which is how the SDK is told where a binary is now that
its public signature carries no binary argument.

The dividing line is what that binary can actually produce. Every state it *can*
reach — a report, `--diff`, the tool filter, a non-zero exit — is driven with it
and with nothing else. Two states it *cannot* reach are covered without
fabricating a CLI to fake them: it never prints a malformed report, and with
`--format json` it never prints nothing. Those are proven by calling the strict
reader directly with the payload a broken or newer build would emit
(`test_contract.py`), and by pointing `NOTIGNORED_BIN` at real unrelated programs
— the misconfiguration a user actually hits — to show the plumbing turns their
stdout into a typed error (`test_errors.py`).
"""

from __future__ import annotations

import os
import subprocess
from pathlib import Path
from typing import Any

import pytest

REPO_ROOT = Path(__file__).resolve().parents[3]

# The suppression the fixtures below carry, and what notignored reports for it.
RUFF_LINE = "url = LONG_URL  # noqa: E501  # the vendor's documented endpoint\n"


@pytest.fixture(scope="session")
def notignored_binary() -> Path:
    """The `notignored` built from this workspace — never one from PATH.

    Built rather than located: an SDK proven against a stale binary in
    `target/debug` would report green for a contract that had already moved.
    """
    build = subprocess.run(
        ["cargo", "build", "--locked", "--quiet", "--bin", "notignored"],
        cwd=REPO_ROOT,
        env={**os.environ, "CARGO_TERM_QUIET": "true"},
        capture_output=True,
        text=True,
        check=False,
    )
    if build.returncode != 0:
        pytest.fail(
            "cannot build the notignored binary these journeys drive:\n"
            f"{build.stderr}\nACTION: run `just bootstrap` from the repository root"
        )
    binary = (
        REPO_ROOT / "target" / "debug" / ("notignored.exe" if os.name == "nt" else "notignored")
    )
    assert binary.is_file(), f"cargo reported success but {binary} is missing"
    return binary


@pytest.fixture(autouse=True)
def use_the_workspace_binary(notignored_binary: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    """Point every call at the binary this workspace built.

    Autouse so no test can silently fall through to a `notignored` that happens
    to be installed on the machine; the tests that are *about* resolution set the
    variable again, and the later `monkeypatch` wins.
    """
    monkeypatch.setenv("NOTIGNORED_BIN", str(notignored_binary))


@pytest.fixture
def tree(tmp_path: Path) -> Path:
    """A small polyglot source tree with one suppression per language."""
    (tmp_path / "app.py").write_text(RUFF_LINE, encoding="utf-8")
    (tmp_path / "widget.ts").write_text(
        "// eslint-disable-next-line no-console -- the CLI prints here\nconsole.log(1);\n",
        encoding="utf-8",
    )
    (tmp_path / "lib.rs").write_text(
        "#[allow(dead_code)] // reachable from the C ABI only\nfn helper() {}\n",
        encoding="utf-8",
    )
    return tmp_path


def report_payload(
    *,
    directive: dict[str, Any] | None = None,
    **overrides: Any,
) -> dict[str, Any]:
    """A decoded report envelope, so a test can bend one field out of contract.

    This is the shape the real CLI emits; the overrides are what a *newer or
    broken* build could emit and this one cannot, which is the only reason it is
    written here rather than read off a real run.
    """
    ignore: dict[str, Any] = {
        "tool": "ruff",
        "scope": "line",
        "rules": ["E501"],
        "reason": "the vendor's documented endpoint",
        "path": "app.py",
        "line": 1,
        "end_line": 1,
        "column": 17,
        "raw": "# noqa: E501",
        "suppressed": {"start_line": 1, "end_line": 1},
    }
    ignore.update(directive or {})
    envelope: dict[str, Any] = {"version": 1, "ignores": [ignore], "errors": []}
    envelope.update(overrides)
    return envelope


@pytest.fixture
def git_repo(tmp_path: Path) -> Path:
    """A real git repository with one commit, ready for a `--diff` comparison."""
    repo = tmp_path / "repo"
    repo.mkdir()

    def git(*args: str) -> None:
        subprocess.run(["git", *args], cwd=repo, check=True, capture_output=True)

    git("init", "--initial-branch=main")
    git("config", "user.email", "tests@example.com")
    git("config", "user.name", "notignored tests")
    (repo / "app.py").write_text("x = 1  # type: ignore[assignment]  # already here\n", "utf-8")
    git("add", "-A")
    git("commit", "-m", "the base commit")
    return repo
