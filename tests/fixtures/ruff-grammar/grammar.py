import os  # NOQA: F401
import sys  # noqa:F401,E402
import json  # noqa: F401 E402
import re  # type: ignore  # noqa: F401  # embedded after another directive
import io  # noqa: F401, oops
import abc  # noqa
NAMES = (os, sys, json, re, io, abc, "# noqa: F811")

# llmlint: ignore-file[suppressions_justified] fixture input, not production code:
# these lines exist to exercise every directive form the parser claims, including
# the reason-less ones, and the e2e asserts each is reported exactly as written.
