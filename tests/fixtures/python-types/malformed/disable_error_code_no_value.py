# `disable-error-code` with no value: plain intent, no effect.

def double(value: int) -> int:
    return value * 2

# mypy: disable-error-code
double("two")

# llmlint: ignore-file[suppressions_justified] the missing reason is the point:
# this fixture is the input that proves notignored reports an unjustified
# suppression, and the parity test asserts its `reason` comes back null.
