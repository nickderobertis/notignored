# llmlint: ignore-file[errors_are_contextualized] this shim hands the caller a bare
# error on purpose
#
# a separate paragraph, which is commentary rather than part of that reason
import sys


def trace(url):
    # llmlint: ignore[no_debug_prints] the trace below is this helper's whole job
        # indented further, so a different thought entirely
    sys.stderr.write(url)  # llmlint: ignore[no_todo_comments] the marker below is data
                           # and this aligned note is separate commentary
    return "# TODO: not a comment"


def probe(url):
    # llmlint: ignore[no_debug_prints] printing here is the point of the helper

    # after a blank line, so not part of the sentence above
    print(url)


def render(rows):
    # llmlint: ignore[no_debug_prints] the dump below is what this helper is for
    # noqa: T201
    print(rows)
