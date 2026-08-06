# llmlint: ignore-file[boundary_inputs_validated] a transport shim: the caller validates before this layer
import os  # noqa: F401  # re-exported for the public API

# llmlint: ignore-block[tool_output_is_signal] the trace is this module's whole job
print("connecting")
print("connected")
# llmlint: ignore-end[tool_output_is_signal]

MESSAGE = "# llmlint: ignore[comments_earn_their_place] inside a string literal"
