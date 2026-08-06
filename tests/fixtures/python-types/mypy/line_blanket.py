# A blanket line directive on the offending line.

def double(value: int) -> int:
    return value * 2


double("two")  # type: ignore
