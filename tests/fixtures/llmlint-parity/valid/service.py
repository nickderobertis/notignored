# llmlint: ignore-file[errors_are_contextualized] a thin transport shim: the caller owns the context
import sys

# llmlint: ignore-block[no_debug_prints] the trace is the feature here, not a leftover
print("connecting")
print("connected")
# llmlint: ignore-end[no_debug_prints]


def fetch(url):
    sys.stderr.write(url)  # llmlint: ignore[no_todo_comments] the marker below is data, not a TODO
    return "# TODO: not a comment"


def trace(url):
    # llmlint: ignore[no_debug_prints] the trace below is this helper's whole job
    print(url)
