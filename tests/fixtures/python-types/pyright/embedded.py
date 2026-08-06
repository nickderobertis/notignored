# A directive that does not open its comment is still the one pyright honours.

def double(value: int) -> int:
    return value * 2


double("two")  # legacy call site  # pyright: ignore[reportArgumentType]
