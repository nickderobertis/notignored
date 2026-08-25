"""Strict parsing of the v1 envelope, at the boundary that does the parsing.

A working `notignored` cannot print any of these payloads — the SDK pins its CLI
exactly, which is what makes strictness safe — so there is no real run that
produces one. Rather than fabricate a CLI that would, these call the real strict
reader with the payload a *newer or broken* build would emit. Nothing is mocked:
the code under test is the code that ships, invoked directly.

The plumbing that carries its verdict out through `scan()` is proven separately,
against a real subprocess, in `test_errors.py`.
"""

from __future__ import annotations

from typing import Any

import pytest
from conftest import report_payload

from notignored_sdk import Change, NotignoredContractError, Scope, Suppressed, Tool
from notignored_sdk._model import report_from_payload


def test_the_shape_the_real_cli_emits_round_trips() -> None:
    """The control: the payload these cases bend is one the reader accepts."""
    report = report_from_payload(report_payload())

    directive = report.ignores[0]
    assert report.version == 1
    assert directive.tool is Tool.RUFF
    assert directive.scope is Scope.LINE
    assert directive.rules == ("E501",)
    assert directive.suppressed == Suppressed(start_line=1, end_line=1)


def test_a_newer_envelope_is_rejected_rather_than_read_short() -> None:
    """A report from a build this SDK does not understand may carry fields we drop."""
    with pytest.raises(NotignoredContractError, match="upgrade notignored-sdk"):
        report_from_payload(report_payload(version=2))


def test_an_older_envelope_asks_for_a_newer_binary() -> None:
    with pytest.raises(NotignoredContractError, match="upgrade the notignored binary"):
        report_from_payload(report_payload(version=0))


def test_an_unknown_tool_is_an_error_and_never_a_dropped_record() -> None:
    with pytest.raises(NotignoredContractError) as raised:
        report_from_payload(report_payload(directive={"tool": "flake8"}))

    assert "'flake8'" in str(raised.value)
    assert "ruff" in str(raised.value)


def test_an_unknown_scope_is_an_error_too() -> None:
    with pytest.raises(NotignoredContractError, match="known scopes"):
        report_from_payload(report_payload(directive={"scope": "paragraph"}))


@pytest.mark.parametrize(
    ("payload", "expected"),
    [
        pytest.param([], "not an object", id="the envelope is an array"),
        pytest.param({"ignores": [], "errors": []}, "no 'version' field", id="no version"),
        pytest.param(
            {"version": "1", "ignores": [], "errors": []}, "not an integer", id="version is text"
        ),
        pytest.param(
            {"version": True, "ignores": [], "errors": []}, "not an integer", id="version is a bool"
        ),
        pytest.param({"version": 1, "errors": []}, "no 'ignores' field", id="no ignores"),
        pytest.param({"version": 1, "ignores": []}, "no 'errors' field", id="no errors"),
        pytest.param(
            {"version": 1, "ignores": {}, "errors": []}, "not an array", id="ignores is an object"
        ),
        pytest.param(
            {"version": 1, "ignores": [1], "errors": []},
            "not an object",
            id="an ignore is a number",
        ),
    ],
)
def test_a_malformed_envelope_names_the_field_that_broke(payload: Any, expected: str) -> None:
    with pytest.raises(NotignoredContractError, match=expected):
        report_from_payload(payload)


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
    field: str, value: Any, expected: str
) -> None:
    with pytest.raises(NotignoredContractError, match=expected):
        report_from_payload(report_payload(directive={field: value}))


def test_a_malformed_report_error_names_the_field_that_broke() -> None:
    with pytest.raises(NotignoredContractError, match=r"errors\[0\] has no 'message'"):
        report_from_payload(report_payload(errors=[{"path": "a.py"}]))


def test_a_field_this_sdk_has_never_heard_of_is_carried_past() -> None:
    """The contract's own rule is that new fields are additive and optional.

    Rejecting one would break this SDK against a CLI that had only added
    something, which is the opposite of the strictness the unknown-tool case is
    protecting.
    """
    report = report_from_payload(
        report_payload(directive={"confidence": "high"}, generated_at="today")
    )

    assert report.ignores[0].tool is Tool.RUFF


def test_a_change_word_this_sdk_does_not_know_is_an_error_not_a_guess() -> None:
    """The same rule an unknown tool gets: a word we cannot read is not one to guess."""
    with pytest.raises(NotignoredContractError, match="which this SDK does not know"):
        report_from_payload(report_payload(directive={"change": "rewritten"}))

    with pytest.raises(NotignoredContractError, match="not a string or null"):
        report_from_payload(report_payload(directive={"change": 3}))


def test_an_absent_change_is_none_rather_than_a_third_value() -> None:
    """A payload without it is what every scan that is not `--diff` produces."""
    assert report_from_payload(report_payload()).ignores[0].change is None
    assert (
        report_from_payload(report_payload(directive={"change": "added"})).ignores[0].change
        is Change.ADDED
    )
