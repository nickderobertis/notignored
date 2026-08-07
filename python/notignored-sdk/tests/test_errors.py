"""Every way a call can fail, and the typed error it fails with.

The spawn and exit branches are driven by the real binary — a path that does not
exist really does make it exit 2 with nothing on stdout. The contract branches
cannot be: a working `notignored` never prints a malformed report, so those are
driven by `stub_binary`, which stands in for a broken or newer CLI.
"""

from __future__ import annotations

import asyncio
from collections.abc import Callable
from pathlib import Path

import pytest
from conftest import report_payload

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

Stub = Callable[..., Path]


def test_a_missing_binary_names_how_to_install_one(tmp_path: Path) -> None:
    missing = tmp_path / "nowhere" / "notignored"

    with pytest.raises(NotignoredNotFoundError) as raised:
        scan(cwd=tmp_path, binary=missing)

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


def test_the_async_form_reports_a_missing_binary_too(tmp_path: Path) -> None:
    with pytest.raises(NotignoredNotFoundError):
        asyncio.run(ascan(cwd=tmp_path, binary=tmp_path / "nowhere" / "notignored"))


def test_a_binary_that_cannot_be_executed_is_a_spawn_error(tmp_path: Path) -> None:
    """A directory is on disk but is not a program; that is not "not installed"."""
    not_a_program = tmp_path / "a-directory"
    not_a_program.mkdir()

    with pytest.raises(NotignoredSpawnError) as raised:
        scan(cwd=tmp_path, binary=not_a_program)

    assert not isinstance(raised.value, NotignoredNotFoundError)
    assert "cannot run" in str(raised.value)


def test_a_scan_that_cannot_run_raises_with_the_clis_own_stderr(
    notignored_binary: Path, tmp_path: Path
) -> None:
    """A path that does not exist: the CLI exits 2 and prints no report at all."""
    with pytest.raises(NotignoredExitError) as raised:
        scan(["does/not/exist"], cwd=tmp_path, binary=notignored_binary)

    assert raised.value.returncode == 2
    assert "does/not/exist" in raised.value.stderr
    assert "hint:" in raised.value.stderr
    assert "does/not/exist" in str(raised.value)


def test_the_async_form_raises_the_same_exit_error(notignored_binary: Path, tmp_path: Path) -> None:
    with pytest.raises(NotignoredExitError) as raised:
        asyncio.run(ascan(["does/not/exist"], cwd=tmp_path, binary=notignored_binary))

    assert raised.value.returncode == 2


def test_output_that_is_not_json_is_a_contract_error(stub_binary: Stub) -> None:
    with pytest.raises(NotignoredContractError, match="not JSON"):
        scan(binary=stub_binary("this is not a report\n"))


def test_a_nonzero_exit_with_unreadable_output_reports_the_exit_not_the_contract(
    stub_binary: Stub,
) -> None:
    """When both went wrong, the exit code is the cause and the parse is the symptom."""
    with pytest.raises(NotignoredExitError) as raised:
        scan(binary=stub_binary("not a report", 3))

    assert raised.value.returncode == 3


def test_a_silent_success_is_a_contract_error(stub_binary: Stub) -> None:
    with pytest.raises(NotignoredContractError, match="printed no report"):
        scan(binary=stub_binary(""))


def test_a_newer_envelope_is_rejected_rather_than_read_short(stub_binary: Stub) -> None:
    """A report from a build this SDK does not understand may carry fields we drop."""
    with pytest.raises(NotignoredContractError, match="upgrade notignored-sdk"):
        scan(binary=stub_binary(report_payload(version=2)))


def test_an_older_envelope_asks_for_a_newer_binary(stub_binary: Stub) -> None:
    with pytest.raises(NotignoredContractError, match="upgrade the notignored binary"):
        scan(binary=stub_binary(report_payload(version=0)))


def test_an_unknown_tool_is_an_error_and_never_a_dropped_record(stub_binary: Stub) -> None:
    with pytest.raises(NotignoredContractError) as raised:
        scan(binary=stub_binary(report_payload(directive={"tool": "flake8"})))

    assert "'flake8'" in str(raised.value)
    assert "ruff" in str(raised.value)


def test_an_unknown_scope_is_an_error_too(stub_binary: Stub) -> None:
    with pytest.raises(NotignoredContractError, match="known scopes"):
        scan(binary=stub_binary(report_payload(directive={"scope": "paragraph"})))


@pytest.mark.parametrize(
    ("payload", "expected"),
    [
        pytest.param("[]", "not an object", id="the envelope is an array"),
        pytest.param('{"ignores": [], "errors": []}', "no 'version' field", id="no version"),
        pytest.param('{"version": "1", "ignores": [], "errors": []}', "not an integer", id="text"),
        pytest.param('{"version": true, "ignores": [], "errors": []}', "not an integer", id="bool"),
        pytest.param('{"version": 1, "errors": []}', "no 'ignores' field", id="no ignores"),
        pytest.param('{"version": 1, "ignores": []}', "no 'errors' field", id="no errors"),
        pytest.param('{"version": 1, "ignores": {}, "errors": []}', "not an array", id="ignores"),
        pytest.param('{"version": 1, "ignores": [1], "errors": []}', "not an object", id="ignore"),
    ],
)
def test_a_malformed_envelope_names_the_field_that_broke(
    stub_binary: Stub, payload: str, expected: str
) -> None:
    with pytest.raises(NotignoredContractError, match=expected):
        scan(binary=stub_binary(payload))


@pytest.mark.parametrize(
    ("field", "value", "expected"),
    [
        pytest.param("rules", "E501", "not an array", id="rules is not an array"),
        pytest.param("rules", [1], r"rules\[0\] is a int", id="a rule is not a string"),
        pytest.param("reason", 7, "not a string or null", id="reason is neither"),
        pytest.param("path", None, "not a string", id="path is null"),
        pytest.param("line", "1", "not an integer", id="line is text"),
        pytest.param("raw", None, "not a string", id="raw is null"),
        pytest.param("suppressed", [], "not an object", id="suppressed is an array"),
        pytest.param("suppressed", {"end_line": 1}, "no 'start_line'", id="no start_line"),
        pytest.param("suppressed", {"start_line": 1}, "no 'end_line'", id="no end_line"),
    ],
)
def test_a_malformed_directive_names_the_field_that_broke(
    stub_binary: Stub, field: str, value: object, expected: str
) -> None:
    with pytest.raises(NotignoredContractError, match=expected):
        scan(binary=stub_binary(report_payload(directive={field: value})))


def test_a_malformed_report_error_names_the_field_that_broke(stub_binary: Stub) -> None:
    with pytest.raises(NotignoredContractError, match=r"errors\[0\] has no 'message'"):
        scan(binary=stub_binary(report_payload(errors=[{"path": "a.py"}])))


def test_a_field_this_sdk_has_never_heard_of_is_carried_past(stub_binary: Stub) -> None:
    """The contract's own rule is that new fields are additive and optional.

    Rejecting one would break this SDK against a CLI that had only added something,
    which is the opposite of the strictness the unknown-tool case is protecting.
    """
    report = scan(
        binary=stub_binary(report_payload(directive={"confidence": "high"}, generated_at="today"))
    )

    assert report.ignores[0].tool is Tool.RUFF


def test_the_error_types_are_one_hierarchy_a_caller_can_catch(tmp_path: Path) -> None:
    with pytest.raises(NotignoredError):
        scan(cwd=tmp_path, binary=tmp_path / "nowhere")


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
    kwargs: dict[str, object], expected: str
) -> None:
    """Validated before a process starts, so a typo never becomes a scan of the wrong tree."""
    with pytest.raises(TypeError, match=expected):
        scan(**kwargs)


def test_an_unknown_tool_name_is_rejected_before_a_process_starts() -> None:
    with pytest.raises(ValueError, match="known tools are"):
        scan(tools=["flake8"])


def test_an_empty_path_is_rejected() -> None:
    with pytest.raises(ValueError, match="is empty"):
        scan([""])
