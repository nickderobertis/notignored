"""What every journey here drives: the real binary this workspace builds.

Nothing in this suite mocks the subprocess or the CLI. `notignored_binary` is
compiled from the crate beside this project, so a report shape that moved in
`src/` fails here on the same commit that moved it — which is the whole reason
the SDK's Nx project depends on the crate's.

`stub_binary` is the one exception, and it is not a mock of the CLI: it stands in
for a *broken or newer* `notignored`, which a working one cannot be. It is how
the contract-error branches — output that is not JSON, an envelope from a future
version, a tool this SDK has never heard of — are driven for real.
"""

from __future__ import annotations

import json
import os
import stat
import subprocess
from collections.abc import Callable
from pathlib import Path

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


@pytest.fixture
def stub_binary(tmp_path: Path) -> Callable[..., Path]:
    """Build an executable that prints `stdout` and exits `code`, whatever its arguments.

    Written per platform rather than as a `#!`-script, because a shebang is not a
    thing Windows runs.
    """

    made: list[Path] = []

    def build(stdout: str, code: int = 0) -> Path:
        payload = tmp_path / f"stub-{len(made)}.json"
        payload.write_text(stdout, encoding="utf-8")
        if os.name == "nt":
            script = tmp_path / f"stub-{len(made)}.cmd"
            script.write_text(
                f"@echo off\r\ntype {payload}\r\nexit /b {code}\r\n",
                encoding="utf-8",
            )
        else:
            script = tmp_path / f"stub-{len(made)}.sh"
            script.write_text(
                f'#!/bin/sh\ncat "{payload}"\nexit {code}\n',
                encoding="utf-8",
            )
            script.chmod(script.stat().st_mode | stat.S_IEXEC | stat.S_IXGRP | stat.S_IXOTH)
        made.append(script)
        return script

    return build


def report_payload(
    *,
    directive: dict[str, object] | None = None,
    **overrides: object,
) -> str:
    """A serialized report envelope, so a test can bend one field out of contract."""
    ignore: dict[str, object] = {
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
    envelope: dict[str, object] = {"version": 1, "ignores": [ignore], "errors": []}
    envelope.update(overrides)
    return json.dumps(envelope)


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
