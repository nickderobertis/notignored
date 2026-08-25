"""The public surface itself: what a caller may import, and nothing more.

The predecessor of this file proved the Nx and packaging wiring while the project
was a scaffold. That wiring still has to hold — an editable install `bootstrap`
never made, a package layout the build backend cannot find — so those assertions
stay; what they are made against is now a real API.
"""

from __future__ import annotations

import enum
import importlib.metadata
import inspect

import notignored_sdk
from notignored_sdk import Scope, Tool, ascan, scan

SURFACE = {
    "Change",
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
}

# The approved signature — `scan(paths=(), *, diff=False, diff_base=None,
# tools=None, cwd=None)` — as (name, kind, default), which is what the contract
# actually fixes: the names, their order, which are keyword-only, and their
# defaults. Annotations are deliberately not compared; the package is typed and
# they are free to be spelled however the checker prefers.
#
# Written as a literal on purpose: this is the one place duplicating a fixed
# contract earns its keep, because deriving it from the code would make the code
# its own specification.
APPROVED_PARAMETERS = [
    ("paths", inspect.Parameter.POSITIONAL_OR_KEYWORD, ()),
    ("diff", inspect.Parameter.KEYWORD_ONLY, False),
    ("diff_base", inspect.Parameter.KEYWORD_ONLY, None),
    ("tools", inspect.Parameter.KEYWORD_ONLY, None),
    ("cwd", inspect.Parameter.KEYWORD_ONLY, None),
]


def test_the_project_is_installed_under_its_published_name() -> None:
    assert importlib.metadata.distribution("notignored-sdk")


def test_the_public_surface_is_the_records_the_errors_and_the_two_entry_points() -> None:
    """Held small on purpose: everything else is an implementation detail."""
    assert set(notignored_sdk.__all__) == SURFACE
    assert {name for name in vars(notignored_sdk) if not name.startswith("_")} == SURFACE


def test_both_entry_points_take_exactly_the_approved_arguments() -> None:
    """No binary argument, no extras: the flags the CLI has and nothing else."""
    for entry_point in (scan, ascan):
        parameters = [
            (parameter.name, parameter.kind, parameter.default)
            for parameter in inspect.signature(entry_point).parameters.values()
        ]
        assert parameters == APPROVED_PARAMETERS, entry_point.__name__


def test_both_entry_points_take_the_identical_arguments() -> None:
    """`ascan` is `scan` on an event loop, so a caller can swap one for the other."""
    assert inspect.signature(ascan) == inspect.signature(scan)
    assert inspect.iscoroutinefunction(ascan)
    assert not inspect.iscoroutinefunction(scan)


def test_the_enums_are_real_strenums() -> None:
    """Not a `(str, Enum)` lookalike, which stringifies as `Tool.RUFF`."""
    assert issubclass(Tool, enum.StrEnum)
    assert issubclass(Scope, enum.StrEnum)
    assert f"{Tool.RUFF}" == "ruff"
    assert f"{Scope.NEXT_LINE}" == "next-line"
    assert "".join([Scope.FILE]) == "file"


def test_the_enums_cover_the_ten_tools_and_the_four_scopes() -> None:
    assert [tool.value for tool in Tool] == [
        "eslint",
        "biome",
        "ruff",
        "typescript",
        "mypy",
        "pyright",
        "ty",
        "rust",
        "shellcheck",
        "llmlint",
    ]
    assert [scope.value for scope in Scope] == ["line", "next-line", "file", "block"]
