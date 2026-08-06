"""Fetch the vendor catalogue."""

import urllib.request

CATALOGUE = urllib.request.urlopen("https://example.invalid/a/very/long/vendor/catalogue.json")  # noqa: E501  # the vendor URL cannot be wrapped


def load():
    return CATALOGUE.read()
