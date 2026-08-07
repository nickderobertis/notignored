"""The billing client, and the suppressions a real one collects."""

import legacy_sdk  # type: ignore[import-untyped]  # the vendored SDK ships no type stubs
from retries import Retry  # noqa: F401  # re-exported so callers can configure retries


def charge(customer: str, cents: int) -> bytes:
    """Charge `customer` through the legacy SDK's session."""
    return legacy_sdk.session().post(f"/charge/{customer}", cents).content


def refund(charge_id: str) -> bytes:
    """Reverse a charge the legacy SDK made."""
    return legacy_sdk.session().post(f"/refund/{charge_id}").content  # noqa: E501  # the SDK builds this path, not us
