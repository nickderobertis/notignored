# Inline config that does not own its line configures nothing.

def double(value: int) -> int:
    return value * 2


double("two")  # mypy: ignore-errors
