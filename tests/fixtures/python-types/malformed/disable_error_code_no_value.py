# `disable-error-code` with no value: plain intent, no effect.

def double(value: int) -> int:
    return value * 2

# mypy: disable-error-code
double("two")
