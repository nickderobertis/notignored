# A coded line directive, with the reason it was needed.

def double(value: int) -> int:
    return value * 2


double("two")  # pyright: ignore[reportArgumentType]  # upstream stub is wrong
