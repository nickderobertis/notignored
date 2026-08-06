# A module-level exemption sits below the code it exempts.

def double(value: int) -> int:
    return value * 2

# mypy: ignore-errors
double("two")

# llmlint: ignore-file[suppressions_justified] fixture input, not production
# code: this file is the reason-less form of the directive, and the parity e2e
# asserts the CLI reports `reason: null` for it. A reason here would delete the
# only coverage of that form.
