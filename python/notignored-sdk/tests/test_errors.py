"""Every way a call can fail, and the typed error it fails with.

All of these go through a real subprocess. The spawn and exit branches use the
real binary — a path that does not exist really does make it exit 2. The two
branches a working `notignored` can never reach use a real, unrelated program on
`NOTIGNORED_BIN`, which is not a fabricated CLI but the misconfiguration a user
actually hits: `coreutils`' `echo` prints something that is not a report, and
`true` prints nothing at all. What each malformed *payload* means is proven
against the reader itself in `test_contract.py`.
"""

from __future__ import annotations

import asyncio
import shutil
from pathlib import Path
from typing import Any, cast

import pytest

from notignored_sdk import (
    NotignoredContractError,
    NotignoredError,
    NotignoredExitError,
    NotignoredNotFoundError,
    NotignoredSpawnError,
    Tool,
    ascan,
    scan,
)

# Real programs, not stand-ins for notignored: one that ignores its arguments and
# prints them back, one that ignores them and prints nothing. Both are how a
# mis-set NOTIGNORED_BIN behaves, which is the journey under test.
ECHO = shutil.which("echo")
TRUE = shutil.which("true")


def test_a_missing_binary_names_how_to_install_one(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    missing = tmp_path / "nowhere" / "notignored"
    monkeypatch.setenv("NOTIGNORED_BIN", str(missing))

    with pytest.raises(NotignoredNotFoundError) as raised:
        scan(cwd=tmp_path)

    assert str(missing) in str(raised.value)
    assert "pip install notignored-sdk" in str(raised.value)
    assert raised.value.command == str(missing)
    assert isinstance(raised.value, NotignoredSpawnError)


def test_nothing_on_path_and_no_override_is_the_same_typed_error(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.delenv("NOTIGNORED_BIN", raising=False)
    monkeypatch.setenv("PATH", str(tmp_path))

    with pytest.raises(NotignoredNotFoundError, match="NOTIGNORED_BIN"):
        scan(cwd=tmp_path)


def test_the_async_form_reports_a_missing_binary_too(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setenv("NOTIGNORED_BIN", str(tmp_path / "nowhere" / "notignored"))

    with pytest.raises(NotignoredNotFoundError):
        asyncio.run(ascan(cwd=tmp_path))


def test_a_binary_that_cannot_be_executed_is_a_spawn_error(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """A directory is on disk but is not a program; that is not "not installed"."""
    not_a_program = tmp_path / "a-directory"
    not_a_program.mkdir()
    monkeypatch.setenv("NOTIGNORED_BIN", str(not_a_program))

    with pytest.raises(NotignoredSpawnError) as raised:
        scan(cwd=tmp_path)

    assert not isinstance(raised.value, NotignoredNotFoundError)
    assert "cannot run" in str(raised.value)


def test_a_scan_that_cannot_run_raises_with_the_clis_own_stderr(tmp_path: Path) -> None:
    """A path that does not exist: the CLI exits 2, and the SDK says so."""
    with pytest.raises(NotignoredExitError) as raised:
        scan(["does/not/exist"], cwd=tmp_path)

    assert raised.value.returncode == 2
    assert "does/not/exist" in raised.value.stderr
    assert "hint:" in raised.value.stderr
    assert "does/not/exist" in str(raised.value)


def test_an_unreadable_file_is_a_nonzero_exit_carrying_the_clis_stderr(tmp_path: Path) -> None:
    """Every non-zero exit is this error, including the one that still printed a report.

    The CLI names the unreadable file on stderr as well as in the report's
    `errors`, so nothing is lost by refusing the exit code — and a caller cannot
    mistake a tree it could not fully read for a clean one.
    """
    (tmp_path / "broken.py").write_bytes(b"x\xff\n")

    with pytest.raises(NotignoredExitError) as raised:
        scan(cwd=tmp_path)

    assert raised.value.returncode == 2
    assert "broken.py" in raised.value.stderr


def test_the_async_form_raises_the_same_exit_error(tmp_path: Path) -> None:
    with pytest.raises(NotignoredExitError) as raised:
        asyncio.run(ascan(["does/not/exist"], cwd=tmp_path))

    assert raised.value.returncode == 2


@pytest.mark.skipif(ECHO is None, reason="needs a real `echo` on PATH")
def test_a_binary_that_is_not_notignored_is_a_contract_error(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """NOTIGNORED_BIN pointed at the wrong program: exit 0, and not a report."""
    monkeypatch.setenv("NOTIGNORED_BIN", str(ECHO))

    with pytest.raises(NotignoredContractError, match="not JSON"):
        scan(cwd=tmp_path)


@pytest.mark.skipif(TRUE is None, reason="needs a real `true` on PATH")
def test_a_binary_that_prints_nothing_is_a_contract_error(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setenv("NOTIGNORED_BIN", str(TRUE))

    with pytest.raises(NotignoredContractError, match="printed no report"):
        scan(cwd=tmp_path)


@pytest.mark.skipif(ECHO is None, reason="needs a real `echo` on PATH")
def test_the_async_form_reports_the_same_contract_error(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setenv("NOTIGNORED_BIN", str(ECHO))

    with pytest.raises(NotignoredContractError, match="not JSON"):
        asyncio.run(ascan(cwd=tmp_path))


def test_the_error_types_are_one_hierarchy_a_caller_can_catch(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setenv("NOTIGNORED_BIN", str(tmp_path / "nowhere"))

    with pytest.raises(NotignoredError):
        scan(cwd=tmp_path)


@pytest.mark.parametrize(
    ("kwargs", "expected"),
    [
        pytest.param({"paths": "src"}, "sequence of paths", id="a bare string of paths"),
        pytest.param({"paths": Path("src")}, "sequence of paths", id="a bare Path"),
        pytest.param({"paths": [b"src"]}, "not a path", id="bytes"),
        pytest.param({"paths": [7]}, "not a path", id="a number"),
        pytest.param({"tools": "ruff"}, "sequence of tools", id="a bare string of tools"),
        pytest.param({"tools": Tool.RUFF}, "sequence of tools", id="a bare Tool"),
        pytest.param({"tools": [7]}, "not a tool", id="a tool that is a number"),
        pytest.param({"diff": True, "diff_base": 7}, "not a git revision", id="a numeric base"),
    ],
)
def test_an_argument_that_is_not_what_it_claims_is_rejected(
    kwargs: dict[str, Any], expected: str
) -> None:
    """Validated before a process starts, so a typo never becomes a scan of the wrong tree."""
    with pytest.raises(TypeError, match=expected):
        cast("Any", scan)(**kwargs)


def test_an_unknown_tool_name_is_rejected_before_a_process_starts() -> None:
    with pytest.raises(ValueError, match="known tools are"):
        scan(tools=["flake8"])


def test_an_empty_path_is_rejected() -> None:
    with pytest.raises(ValueError, match="is empty"):
        scan([""])
