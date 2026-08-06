"""A small API client, carrying the suppressions a real one tends to collect."""

import legacy_sdk  # type: ignore[import-untyped]  # the vendored SDK ships no type stubs
from retries import Retry  # noqa: F401  # re-exported so callers can configure retries


def fetch(path: str) -> bytes:
    """Read `path` through the legacy SDK's session."""
    return legacy_sdk.session().get(path).content
