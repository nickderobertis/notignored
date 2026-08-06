# llmlint: ignore-file[errors_are_contextualized] a transport shim: the caller adds context
import os  # noqa: F401  # re-exported for the public API

# llmlint: ignore-block[no_debug_prints] the trace is this module's whole job
print("connecting")
print("connected")
# llmlint: ignore-end[no_debug_prints]

MESSAGE = "# llmlint: ignore[no_todo_comments] inside a string literal"
