# A reason after the override is item-list text pyright refuses.

def double(value: int) -> int:
    return value * 2

# pyright: reportArgumentType=false  # upstream stub is wrong
double("two")
