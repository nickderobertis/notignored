"""The typed errors this SDK raises, and nothing else.

Every failure a caller can hit crosses the same boundary: the CLI could not be
started, it refused to finish, or what it printed is not the report contract this
SDK reads. Each of those is a distinct class so a caller can tell "notignored is
not installed" from "the scan itself failed" without matching on a message.
"""

from __future__ import annotations

# The concrete next action for a host with no `notignored` on it. Attached to
# every not-found error because that error is nearly always a setup problem, and
# the traceback is the only place the reader is looking.
INSTALL_HINT = (
    "install it with `pip install notignored-sdk` (which depends on the "
    "notignored-cli binary wheel) or `npm install -g notignored-cli`, or set "
    "NOTIGNORED_BIN to point at one you already have"
)


class NotignoredError(Exception):
    """Base class for every error this SDK raises."""


class NotignoredSpawnError(NotignoredError):
    """The `notignored` subprocess could not be started."""

    def __init__(self, command: str, message: str) -> None:
        self.command = command
        super().__init__(message)


class NotignoredNotFoundError(NotignoredSpawnError):
    """No `notignored` binary could be found to run.

    A subclass of :class:`NotignoredSpawnError` because it is the same failure —
    the process never started — narrowed to the one cause worth its own `except`.
    """


class NotignoredExitError(NotignoredError):
    """`notignored` exited non-zero without printing a report.

    A scan that *does* produce a report is returned even when the CLI exits
    non-zero: unreadable files are part of the contract (``Report.errors``), not
    an SDK-level failure. This is the case where there is no report at all — a
    path that does not exist, an argument the CLI rejected.
    """

    def __init__(self, returncode: int, stderr: str) -> None:
        self.returncode = returncode
        self.stderr = stderr
        super().__init__(f"notignored exited {returncode}: {stderr.strip() or '(no stderr)'}")


class NotignoredContractError(NotignoredError):
    """What `notignored` printed is not the report contract this SDK reads.

    Parsing is strict on purpose — an unknown tool, an unknown scope, or a
    missing field is this error rather than a silently dropped record, because a
    scan that quietly reports fewer suppressions than it found is worse than one
    that fails. The SDK depends on an exact `notignored-cli`, so in a supported
    install this cannot happen.
    """
