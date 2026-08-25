# llmlint: ignore-file[errors_are_contextualized] a thin transport shim: the caller
# knows which request it made, and a wrapper added here would only guess at the
# context it already has
import sys


def trace(url):
    # llmlint: ignore[no_debug_prints] the trace below is this helper's whole job,
    # and a logger in its place would need configuration the caller does not
    # have
    sys.stderr.write(url)
