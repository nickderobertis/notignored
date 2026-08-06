# A coded line directive, with the reason it was needed.

def double(value: int) -> int:
    return value * 2


double("two")  # ty: ignore[invalid-argument-type]  # upstream stub is wrong
