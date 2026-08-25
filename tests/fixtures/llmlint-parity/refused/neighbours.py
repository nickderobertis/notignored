def fetch(url):
    # llmlint: ignore[no_debug_prints] the trace below is this helper's whole job
    # noqa: T201
    print(url)


def widen(value):
    # llmlint: ignore[errors_are_contextualized] the caller owns this call's context
    # type: ignore[arg-type]
    return value + 1


def narrow(value):
    # llmlint: ignore[errors_are_contextualized] the caller owns this one's too
    # pyright: ignore[reportGeneralTypeIssues]
    return value - 1


def probe(value):
    # llmlint: ignore[errors_are_contextualized] and the context here is the caller's
    # ty: ignore[invalid-argument-type]
    return value * 2
