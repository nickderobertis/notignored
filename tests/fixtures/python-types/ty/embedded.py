# A directive that does not open its comment is still the one ty honours.

def double(value: int) -> int:
    return value * 2


double("two")  # legacy call site  # ty: ignore[invalid-argument-type]
