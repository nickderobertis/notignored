"""The one entry point, in both forms: `scan` and `ascan`.

The SDK is a thin, typed shell over the real `notignored` binary — it never
re-implements a parser, and it never shells out to anything else. `scan` and
`ascan` take the same arguments and return the same :class:`Report`; the only
difference is which subprocess API runs the process, so `pytest` can call one
directly and an async service the other.

Arguments are validated *before* a process is spawned, and paths go after a `--`
separator so a filename that starts with a dash can never be read as a flag.

Which binary runs is not an argument: `NOTIGNORED_BIN` is the explicit override
and `PATH` is the fallback, so the two entry points carry exactly the flags the
CLI has and nothing else.
"""

from __future__ import annotations

import asyncio
import json
import os
import shutil
import subprocess
from collections.abc import Sequence
from typing import Any, Union

from ._errors import (
    INSTALL_HINT,
    NotignoredContractError,
    NotignoredExitError,
    NotignoredNotFoundError,
    NotignoredSpawnError,
)
from ._model import Report, Tool, report_from_payload

#: Names the `notignored` to run. The public signature carries no binary
#: argument — this is the explicit override, and it wins over ``PATH``.
BINARY_ENV_VAR = "NOTIGNORED_BIN"

#: Anything `os.fspath` accepts: `str`, `pathlib.Path`, or another path-like.
PathLike = Union[str, "os.PathLike[str]"]

# A report of a large tree is a single JSON document on one line, so the async
# reader's default 64 KiB line limit is far too small.
_STREAM_LIMIT = 64 * 1024 * 1024


def _resolve_binary() -> str:
    """Which `notignored` this call runs: the env override, then the one on PATH."""
    override = os.environ.get(BINARY_ENV_VAR)
    if override:
        return override
    found = shutil.which("notignored")
    if found is None:
        raise NotignoredNotFoundError(
            "notignored",
            f"no `notignored` binary on PATH and {BINARY_ENV_VAR} is unset; {INSTALL_HINT}",
        )
    return found


def _paths(paths: Sequence[PathLike]) -> list[str]:
    """The positional PATHS, validated as a sequence of real paths."""
    if isinstance(paths, (str, bytes, os.PathLike)):
        raise TypeError("paths is a sequence of paths; pass [path] to scan a single one")
    resolved = []
    for index, path in enumerate(paths):
        if isinstance(path, bytes) or not isinstance(path, (str, os.PathLike)):
            raise TypeError(f"paths[{index}] is {type(path).__name__}, not a path")
        text = os.fspath(path)
        if not text:
            raise ValueError(f"paths[{index}] is empty; omit it to scan the current directory")
        resolved.append(text)
    return resolved


def _tools(tools: Sequence[Tool | str] | None) -> list[str]:
    """The `--tool` filter, rejecting an unknown name before a process is spawned."""
    if tools is None:
        return []
    if isinstance(tools, (str, bytes, Tool)):
        raise TypeError("tools is a sequence of tools; pass [tool] to filter to a single one")
    names = []
    for index, tool in enumerate(tools):
        if not isinstance(tool, str):
            raise TypeError(f"tools[{index}] is {type(tool).__name__}, not a tool or its name")
        try:
            names.append(Tool(tool).value)
        except ValueError:
            known = ", ".join(known_tool.value for known_tool in Tool)
            raise ValueError(f"tools[{index}] is {tool!r}; known tools are {known}") from None
    return names


def _command(
    paths: Sequence[PathLike],
    diff: bool,
    diff_base: str | None,
    tools: Sequence[Tool | str] | None,
) -> list[str]:
    """The argument vector this call implies.

    Every argument is validated before the binary is even resolved, so a typo in
    a call is a `TypeError`/`ValueError` about the typo rather than whatever the
    host happens to have installed.
    """
    if diff_base is not None:
        if not diff:
            raise ValueError("diff_base needs diff=True; there is nothing to compare it against")
        if not isinstance(diff_base, str):
            raise TypeError(f"diff_base is {type(diff_base).__name__}, not a git revision")
    selected = _paths(paths)
    names = _tools(tools)
    argv = [_resolve_binary(), "--format", "json"]
    for name in names:
        argv.extend(("--tool", name))
    if diff:
        argv.append("--diff")
    if diff_base is not None:
        argv.extend(("--diff-base", diff_base))
    # `--` last: a path spelled `-x` is a path, not a flag this SDK forwarded.
    if selected:
        argv.append("--")
        argv.extend(selected)
    return argv


def _spawn_failure(command: str, error: OSError) -> NotignoredSpawnError:
    """The OS's refusal to start the process, as one of this SDK's errors."""
    if isinstance(error, FileNotFoundError):
        return NotignoredNotFoundError(
            command, f"no `notignored` binary at {command!r}; {INSTALL_HINT}"
        )
    return NotignoredSpawnError(command, f"cannot run `notignored` at {command!r}: {error}")


def _report(returncode: int, stdout: bytes, stderr: bytes) -> Report:
    """One finished run, as a report or as the error that stopped it.

    Every non-zero exit is a :class:`NotignoredExitError` carrying the CLI's own
    stderr — including the exit 2 it uses for a file it could not read, which it
    reports on stderr as well as in the report's `errors`. Only a clean run
    returns, so `Report.errors` is empty in everything handed back here.
    """
    if returncode != 0:
        raise NotignoredExitError(returncode, stderr.decode("utf-8", errors="replace"))
    if not stdout.strip():
        raise NotignoredContractError("notignored printed no report")
    try:
        payload: Any = json.loads(stdout)
    except (UnicodeDecodeError, json.JSONDecodeError) as error:
        message = f"notignored printed output that is not JSON: {error}"
        raise NotignoredContractError(message) from error
    return report_from_payload(payload)


def scan(
    paths: Sequence[PathLike] = (),
    *,
    diff: bool = False,
    diff_base: str | None = None,
    tools: Sequence[Tool | str] | None = None,
    cwd: PathLike | None = None,
) -> Report:
    """Report every suppression comment `notignored` finds.

    :param paths: Files and/or directories to scan. Directories are walked
        recursively, honouring `.gitignore`. Empty scans the working directory,
        which is the CLI's own default.
    :param diff: Report only the suppressions this change added.
    :param diff_base: The git revision `diff` compares against. Passing it
        without ``diff=True`` is a :class:`ValueError`, exactly as the CLI
        rejects ``--diff-base`` without ``--diff``.
    :param tools: Report only these tools; ``None`` reports all of them.
    :param cwd: Directory to run in. Report paths are relative to it.
    :raises NotignoredNotFoundError: No `notignored` binary could be run. Set
        ``NOTIGNORED_BIN`` to name one explicitly.
    :raises NotignoredExitError: The CLI exited non-zero; carries its stderr.
    :raises NotignoredContractError: The report is not the contract this SDK reads.
    """
    argv = _command(paths, diff, diff_base, tools)
    try:
        completed = subprocess.run(  # noqa: S603  # a vector this module built, never a shell
            argv,
            cwd=None if cwd is None else os.fspath(cwd),
            capture_output=True,
            check=False,
        )
    except OSError as error:
        raise _spawn_failure(argv[0], error) from error
    return _report(completed.returncode, completed.stdout, completed.stderr)


async def ascan(
    paths: Sequence[PathLike] = (),
    *,
    diff: bool = False,
    diff_base: str | None = None,
    tools: Sequence[Tool | str] | None = None,
    cwd: PathLike | None = None,
) -> Report:
    """Await :func:`scan`'s result without blocking the event loop.

    Same arguments, same report, same errors; see :func:`scan`.
    """
    argv = _command(paths, diff, diff_base, tools)
    try:
        process = await asyncio.create_subprocess_exec(
            *argv,
            cwd=None if cwd is None else os.fspath(cwd),
            stdout=asyncio.subprocess.PIPE,
            stderr=asyncio.subprocess.PIPE,
            limit=_STREAM_LIMIT,
        )
    except OSError as error:
        raise _spawn_failure(argv[0], error) from error
    stdout, stderr = await process.communicate()
    return _report(process.returncode or 0, stdout, stderr)
