"""Module docstring mentioning # noqa: E501, which is not a directive."""

import os  # noqa: F401  # re-exported for the public API

URL = "https://example.com/a/very/long/path/that/wraps"  # noqa: E501  # long wrapped URL

MESSAGE = "# noqa: E722"


def handler():  # noqa
    return os.getcwd()

# llmlint: ignore-file[suppressions_justified] fixture input, not production code: the bare directive above proves a blanket, reason-less suppression is reported with empty rules and a null reason (tests/golden/report.json).
