"""Grouping fixture: twenty-five identical suppressions among three distinct ones."""

import os  # noqa: F401  # re-exported for the plugin API
import re  # noqa: F811  # the shim redefines it on purpose
URL_1 = "https://example.invalid/1"  # noqa: E501  # the vendor URL cannot be wrapped
URL_2 = "https://example.invalid/2"  # noqa: E501  # the vendor URL cannot be wrapped
URL_3 = "https://example.invalid/3"  # noqa: E501  # the vendor URL cannot be wrapped
URL_4 = "https://example.invalid/4"  # noqa: E501  # the vendor URL cannot be wrapped
URL_5 = "https://example.invalid/5"  # noqa: E501  # the vendor URL cannot be wrapped
URL_6 = "https://example.invalid/6"  # noqa: E501  # the vendor URL cannot be wrapped
URL_7 = "https://example.invalid/7"  # noqa: E501  # the vendor URL cannot be wrapped
URL_8 = "https://example.invalid/8"  # noqa: E501  # the vendor URL cannot be wrapped
URL_9 = "https://example.invalid/9"  # noqa: E501  # the vendor URL cannot be wrapped
URL_10 = "https://example.invalid/10"  # noqa: E501  # the vendor URL cannot be wrapped
URL_11 = "https://example.invalid/11"  # noqa: E501  # the vendor URL cannot be wrapped
URL_12 = "https://example.invalid/12"  # noqa: E501  # the vendor URL cannot be wrapped
URL_13 = "https://example.invalid/13"  # noqa: E501  # the vendor URL cannot be wrapped
URL_14 = "https://example.invalid/14"  # noqa: E501  # the vendor URL cannot be wrapped
URL_15 = "https://example.invalid/15"  # noqa: E501  # the vendor URL cannot be wrapped
URL_16 = "https://example.invalid/16"  # noqa: E501  # the vendor URL cannot be wrapped
URL_17 = "https://example.invalid/17"  # noqa: E501  # the vendor URL cannot be wrapped
URL_18 = "https://example.invalid/18"  # noqa: E501  # the vendor URL cannot be wrapped
URL_19 = "https://example.invalid/19"  # noqa: E501  # the vendor URL cannot be wrapped
URL_20 = "https://example.invalid/20"  # noqa: E501  # the vendor URL cannot be wrapped
URL_21 = "https://example.invalid/21"  # noqa: E501  # the vendor URL cannot be wrapped
URL_22 = "https://example.invalid/22"  # noqa: E501  # the vendor URL cannot be wrapped
URL_23 = "https://example.invalid/23"  # noqa: E501  # the vendor URL cannot be wrapped
URL_24 = "https://example.invalid/24"  # noqa: E501  # the vendor URL cannot be wrapped
URL_25 = "https://example.invalid/25"  # noqa: E501  # the vendor URL cannot be wrapped
import sys  # noqa: E402  # imported after the path is patched
