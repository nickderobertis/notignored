# An own-line directive covers the line below it.

def double(value: int) -> int:
    return value * 2

# ty: ignore[invalid-argument-type]  # the call below is deliberately wrong
double("two")
