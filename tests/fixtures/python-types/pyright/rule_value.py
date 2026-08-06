# A per-file rule override, in the placement pyright documents.

def double(value: int) -> int:
    return value * 2

# pyright: reportArgumentType=false
double("two")

# llmlint: ignore-file[suppressions_justified] fixture input, not production
# code: pyright reads the rest of this line as its item list, so the form cannot
# carry a reason at all and the parity e2e asserts `reason: null` for it. A
# reason here would break the directive the fixture exists to prove.
