# The unsuppressed control: mypy flags the call on the last line.

def double(value: int) -> int:
    return value * 2


double("two")
