# An override that does not open its comment is prose to pyright.

def double(value: int) -> int:
    return value * 2

# legacy call site  # pyright: reportArgumentType=false
double("two")
