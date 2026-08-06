# ruff: noqa: E501  # vendored upstream, not ours to reformat
# mypy: disable-error-code="arg-type, index"

TABLE = {"a": 1}

# llmlint: ignore-file[suppressions_justified] fixture input, not production
# code: mypy reads a trailing comment on a `# mypy:` line as part of the option
# value, so the form has no reason to carry and the e2e asserts `reason: null`
# for it.
