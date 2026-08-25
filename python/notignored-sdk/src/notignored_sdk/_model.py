"""The report contract, as Python objects, and the strict reader that builds them.

This mirrors notignored's `src/model.rs` field for field: it is a **versioned
wire contract**, so the names here are the names on the wire and changing one is
a breaking change for both sides at once.

The reader is strict about what the contract *specifies* — an unknown tool, an
unknown scope, a missing field, or a wrong type is a
:class:`~notignored_sdk.NotignoredContractError` — and tolerant of keys it has
never heard of, because the contract's own rule is that new fields are optional
and additive. Rejecting those would break this SDK against a CLI that had only
added something.
"""

from __future__ import annotations

from dataclasses import dataclass
from enum import StrEnum
from typing import Any

from ._errors import NotignoredContractError

# The one envelope version this SDK reads. Bumped in step with notignored's
# `REPORT_VERSION`; anything else is rejected at the boundary rather than parsed
# into a record that may have lost fields.
SUPPORTED_REPORT_VERSION = 1


class Tool(StrEnum):
    """A lint or type-check tool whose suppression comments notignored parses.

    The values are what the CLI writes in a report and takes on ``--tool``, so a
    plain string is accepted anywhere a :class:`Tool` is.
    """

    ESLINT = "eslint"
    BIOME = "biome"
    RUFF = "ruff"
    TYPESCRIPT = "typescript"
    MYPY = "mypy"
    PYRIGHT = "pyright"
    TY = "ty"
    RUST = "rust"
    SHELLCHECK = "shellcheck"
    LLMLINT = "llmlint"


class Scope(StrEnum):
    """How far a directive's suppression reaches."""

    LINE = "line"
    NEXT_LINE = "next-line"
    FILE = "file"
    BLOCK = "block"


class Change(StrEnum):
    """What a ``--diff`` scan's change did to a suppression.

    ``JUSTIFICATION_EDITED`` says the *justification* moved and nothing else
    did; a directive whose rules or scope the change altered is ``ADDED``,
    because it now silences something its base version did not.
    """

    ADDED = "added"
    JUSTIFICATION_EDITED = "justification-edited"


@dataclass(frozen=True)
class Suppressed:
    """The range of source lines a directive silences."""

    start_line: int
    """First 1-based line the directive silences."""

    end_line: int | None
    """Last 1-based line, or ``None`` when the range runs to end-of-file or is
    unterminated."""


@dataclass(frozen=True)
class IgnoreDirective:
    """One parsed suppression comment."""

    tool: Tool
    """The tool whose rules are being silenced."""

    scope: Scope
    """How far the suppression reaches."""

    rules: tuple[str, ...]
    """Rule names/codes exactly as written. Empty means a blanket suppression of
    every rule the tool would apply."""

    reason: str | None
    """The stated justification, or ``None`` when none was given."""

    path: str
    """Path to the file, relative to the invocation directory, ``/``-separated."""

    line: int
    """1-based line the directive starts on."""

    end_line: int
    """1-based line the directive ends on."""

    column: int
    """1-based column the directive starts at."""

    raw: str
    """The directive exactly as it appears in the source, delimiters included."""

    suppressed: Suppressed
    """The range of lines this directive silences."""

    change: Change | None
    """Whether the change introduced this suppression or rewrote the
    justification of one that already existed, on a ``--diff`` scan.

    ``None`` on any scan that is not a ``--diff`` one: a tree scan has no base,
    so there is nothing to have been added or edited against."""


@dataclass(frozen=True)
class ReportError:
    """A file that could not be read, or a directive that could not be parsed."""

    path: str
    """Path the problem was found at, ``/``-separated."""

    message: str
    """What went wrong, in one line."""


@dataclass(frozen=True)
class Report:
    """The report envelope: everything one scan produced."""

    version: int
    """Envelope version; always :data:`SUPPORTED_REPORT_VERSION` once parsed."""

    ignores: tuple[IgnoreDirective, ...]
    """Every directive found, ordered by path, then line, then column."""

    errors: tuple[ReportError, ...]
    """Files that could not be read and directives that could not be parsed."""


def _object(value: Any, where: str) -> dict[str, Any]:
    """`value` as a JSON object, or a contract error naming what it was instead."""
    if not isinstance(value, dict):
        raise NotignoredContractError(f"{where} is {_kind(value)}, not an object")
    return value


def _kind(value: Any) -> str:
    """How to name a JSON value in a diagnostic."""
    return "null" if value is None else f"a {type(value).__name__}"


def _field(obj: dict[str, Any], key: str, where: str) -> Any:
    if key not in obj:
        raise NotignoredContractError(f"{where} has no {key!r} field")
    return obj[key]


def _text(obj: dict[str, Any], key: str, where: str) -> str:
    value = _field(obj, key, where)
    if not isinstance(value, str):
        raise NotignoredContractError(f"{where}.{key} is {_kind(value)}, not a string")
    return value


def _optional_text(obj: dict[str, Any], key: str, where: str) -> str | None:
    value = _field(obj, key, where)
    if value is None or isinstance(value, str):
        return value
    raise NotignoredContractError(f"{where}.{key} is {_kind(value)}, not a string or null")


def _number(obj: dict[str, Any], key: str, where: str) -> int:
    value = _field(obj, key, where)
    # `bool` is a subclass of `int`, and `true` is not a line number.
    if not isinstance(value, int) or isinstance(value, bool):
        raise NotignoredContractError(f"{where}.{key} is {_kind(value)}, not an integer")
    return value


def _optional_number(obj: dict[str, Any], key: str, where: str) -> int | None:
    if _field(obj, key, where) is None:
        return None
    return _number(obj, key, where)


def _texts(obj: dict[str, Any], key: str, where: str) -> tuple[str, ...]:
    value = _field(obj, key, where)
    if not isinstance(value, list):
        raise NotignoredContractError(f"{where}.{key} is {_kind(value)}, not an array")
    for index, item in enumerate(value):
        if not isinstance(item, str):
            raise NotignoredContractError(f"{where}.{key}[{index}] is {_kind(item)}, not a string")
    return tuple(value)


def _objects(obj: dict[str, Any], key: str, where: str) -> list[dict[str, Any]]:
    value = _field(obj, key, where)
    if not isinstance(value, list):
        raise NotignoredContractError(f"{where}.{key} is {_kind(value)}, not an array")
    return [_object(item, f"{where}.{key}[{index}]") for index, item in enumerate(value)]


def _tool(obj: dict[str, Any], where: str) -> Tool:
    name = _text(obj, "tool", where)
    try:
        return Tool(name)
    except ValueError:
        known = ", ".join(tool.value for tool in Tool)
        raise NotignoredContractError(
            f"{where}.tool is {name!r}, which this SDK does not know (known tools: {known}); "
            "upgrade notignored-sdk"
        ) from None


def _scope(obj: dict[str, Any], where: str) -> Scope:
    name = _text(obj, "scope", where)
    try:
        return Scope(name)
    except ValueError:
        known = ", ".join(scope.value for scope in Scope)
        raise NotignoredContractError(
            f"{where}.scope is {name!r}, which this SDK does not know (known scopes: {known}); "
            "upgrade notignored-sdk"
        ) from None


def _change(obj: dict[str, Any], where: str) -> Change | None:
    """The ``change`` a ``--diff`` scan wrote, or ``None``.

    Absent is not a third value — it says the scan had no base to classify
    against — so a missing key reads as ``None``, exactly the way an unstated
    ``reason`` does. A word this SDK does not know is rejected for the same
    reason an unknown tool is: guessing at it would be reporting a suppression
    as something it is not.
    """
    name = obj.get("change")
    if name is None:
        return None
    if not isinstance(name, str):
        raise NotignoredContractError(f"{where}.change is {_kind(name)}, not a string or null")
    try:
        return Change(name)
    except ValueError:
        known = ", ".join(change.value for change in Change)
        raise NotignoredContractError(
            f"{where}.change is {name!r}, which this SDK does not know (known: {known}); "
            "upgrade notignored-sdk"
        ) from None


def _suppressed(obj: dict[str, Any], where: str) -> Suppressed:
    nested = _object(_field(obj, "suppressed", where), f"{where}.suppressed")
    return Suppressed(
        start_line=_number(nested, "start_line", f"{where}.suppressed"),
        end_line=_optional_number(nested, "end_line", f"{where}.suppressed"),
    )


def _directive(obj: dict[str, Any], where: str) -> IgnoreDirective:
    return IgnoreDirective(
        tool=_tool(obj, where),
        scope=_scope(obj, where),
        rules=_texts(obj, "rules", where),
        reason=_optional_text(obj, "reason", where),
        path=_text(obj, "path", where),
        line=_number(obj, "line", where),
        end_line=_number(obj, "end_line", where),
        column=_number(obj, "column", where),
        raw=_text(obj, "raw", where),
        suppressed=_suppressed(obj, where),
        change=_change(obj, where),
    )


def report_from_payload(payload: Any) -> Report:
    """Read a decoded `--format json` payload as a :class:`Report`, strictly."""
    envelope = _object(payload, "the report")
    version = _number(envelope, "version", "the report")
    if version != SUPPORTED_REPORT_VERSION:
        upgrade = (
            "notignored-sdk" if version > SUPPORTED_REPORT_VERSION else "the notignored binary"
        )
        raise NotignoredContractError(
            f"the report claims version {version}, but this SDK reads version "
            f"{SUPPORTED_REPORT_VERSION}; upgrade {upgrade}"
        )
    return Report(
        version=version,
        ignores=tuple(
            _directive(item, f"the report.ignores[{index}]")
            for index, item in enumerate(_objects(envelope, "ignores", "the report"))
        ),
        errors=tuple(
            ReportError(
                path=_text(item, "path", f"the report.errors[{index}]"),
                message=_text(item, "message", f"the report.errors[{index}]"),
            )
            for index, item in enumerate(_objects(envelope, "errors", "the report"))
        ),
    )
