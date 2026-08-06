# The same exemption, narrowed to named error codes.

def double(value: int) -> int:
    return value * 2

# mypy: disable-error-code="arg-type, index"
double("two")
