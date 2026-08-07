"""The public surface itself: what a caller may import, and nothing more.

The predecessor of this file proved the Nx and packaging wiring while the project
was a scaffold. That wiring still has to hold — an editable install `bootstrap`
never made, a package layout the build backend cannot find — so those assertions
stay; what they are made against is now a real API.
"""

from __future__ import annotations

import importlib.metadata
import inspect

import notignored_sdk

SURFACE = {
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


def test_the_project_is_installed_under_its_published_name() -> None:
    assert importlib.metadata.distribution("notignored-sdk")


def test_the_public_surface_is_the_records_the_errors_and_the_two_entry_points() -> None:
    """Held small on purpose: everything else is an implementation detail."""
    assert set(notignored_sdk.__all__) == SURFACE
    assert {name for name in vars(notignored_sdk) if not name.startswith("_")} == SURFACE


def test_both_entry_points_take_the_identical_arguments() -> None:
    """`ascan` is `scan` on an event loop, so a caller can swap one for the other."""
    assert inspect.signature(notignored_sdk.ascan) == inspect.signature(notignored_sdk.scan)
    assert inspect.iscoroutinefunction(notignored_sdk.ascan)
    assert not inspect.iscoroutinefunction(notignored_sdk.scan)
