# A module-level exemption sits below the code it exempts.

def double(value: int) -> int:
    return value * 2

# mypy: ignore-errors
double("two")
