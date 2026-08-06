# llmlint: ignore-file[boundary_inputs_validated] a transport shim: the gateway validates every field before this layer

# pyright: reportMissingImports=false
import legacy_gateway  # type: ignore[import-not-found]  # no stubs published  # noqa: F401  # imported for its side effects

MESSAGE = "# noqa: E722 inside a string literal"


# ty: ignore[invalid-argument-type]  # the gateway stub types the record id as a str
def fetch(record_id: int) -> str:
    return legacy_gateway.fetch(record_id)  # pyright: ignore[reportAny]  # the SDK returns Any

# llmlint: ignore-file[suppressions_justified] fixture input, not production
# code: pyright reads the rest of a `<rule>=<value>` line as its own item list,
# so that form cannot carry a reason at all and the golden report pins it as
# `reason: null`.
