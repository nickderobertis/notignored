# A directive that does not open its comment is still the one pyright honours.

def double(value: int) -> int:
    return value * 2


double("two")  # legacy call site  # pyright: ignore[reportArgumentType]

# llmlint: ignore-file[suppressions_justified] the missing reason is the point:
# this fixture is the input that proves notignored reports an unjustified
# suppression, and the parity test asserts its `reason` comes back null.
