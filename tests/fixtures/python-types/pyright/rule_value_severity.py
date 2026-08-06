# A rule moved to another severity is configuration, not a suppression.

def double(value: int) -> int:
    return value * 2

# pyright: reportArgumentType=warning
double("two")
