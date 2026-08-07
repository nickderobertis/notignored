"""The placeholder tier: it proves the wiring, not an SDK surface.

Until the SDK lands, what can go wrong here is the plumbing — an editable install
the `bootstrap` target never made, a package layout `hatchling` cannot find, a
`test` target Nx runs from the wrong directory. Importing the distribution the
project actually built is the smallest thing that fails on all three.
"""

import importlib.metadata

import notignored_sdk


def test_the_built_distribution_imports() -> None:
    assert notignored_sdk.__all__ == []
    assert notignored_sdk.__doc__


def test_the_project_is_installed_under_its_published_name() -> None:
    assert importlib.metadata.distribution("notignored-sdk")
