# ruff: noqa

VENDORED_TABLE = {
    "a": 1,
    "b": 2,
}

# llmlint: ignore-file[suppressions_justified] fixture input, not production code:
# the reason-less file-wide directive on line 1 is what proves a blanket
# suppression renders as "(all rules)" (tests/golden/markdown/count-4.md).
