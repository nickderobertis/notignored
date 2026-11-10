"""Grouping fixture: directives that test what makes two suppressions identical."""

import json  # noqa: E501, F401  # re-exported for the plugin API
import os  # noqa: F401,E501  # re-exported for the plugin API
import re  # noqa:E501,F401,E501   #   re-exported   for the plugin API
import sys  # noqa  # re-exported for the plugin API
import io  # noqa: F401  # re-exported for the plugin API
import abc  # noqa: e501, F401  # re-exported for the plugin API
import csv  # noqa: E501, F401  # re-exported for the public API
# ruff: noqa: E501, F401  # re-exported for the plugin API

LIMIT = json.MAX  # type: ignore[attr-defined]  # the stub predates this constant
DEPTH = json.DEPTH  # ty: ignore[attr-defined]  # the stub predates this constant
