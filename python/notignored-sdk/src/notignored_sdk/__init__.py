"""Typed Python access to the `notignored` CLI.

One entry point, in two forms — :func:`scan` for the synchronous callers this
exists for (a pytest suite guarding that nobody silenced a linter to make the
gate pass) and :func:`ascan` for an event loop. Both drive the real `notignored`
binary as a subprocess and return the same strictly-parsed :class:`Report`.

    >>> from notignored_sdk import scan
    >>> report = scan(["src"], tools=["ruff"])          # doctest: +SKIP
    >>> [(d.path, d.line, d.rules) for d in report.ignores]   # doctest: +SKIP
    [('src/app.py', 12, ('E501',))]

The distribution depends on the exact `notignored-cli` it was released with, so
`pip install notignored-sdk` brings a binary that matches this contract.
"""

from ._client import ascan, scan
from ._errors import (
    NotignoredContractError,
    NotignoredError,
    NotignoredExitError,
    NotignoredNotFoundError,
    NotignoredSpawnError,
)
from ._model import IgnoreDirective, Report, ReportError, Scope, Suppressed, Tool

__all__ = [
    "IgnoreDirective",
    "NotignoredContractError",
    "NotignoredError",
    "NotignoredExitError",
    "NotignoredNotFoundError",
    "NotignoredSpawnError",
    "Report",
    "ReportError",
    "Scope",
    "Suppressed",
    "Tool",
    "ascan",
    "scan",
]
