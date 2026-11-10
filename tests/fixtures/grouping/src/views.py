"""Grouping fixture: the same decisions again, in a second file."""

from . import models  # noqa: F401, E501  # re-exported for the plugin API

WIDTH = models.json.WIDTH  # type: ignore[attr-defined]  # the stub predates this constant
_cache = models._cache  # pyright: ignore[reportPrivateUsage]  # the private cache is ours
