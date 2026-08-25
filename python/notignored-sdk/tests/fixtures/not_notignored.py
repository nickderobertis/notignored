#!/usr/bin/env python3
"""A program that is not `notignored`, for the branches only another build can
reach.

Not a stand-in for the real one: every journey that asks what a scan *finds*
drives the compiled binary, and nothing else may stand in for it. This is the
branch where the resolved command turns out to be a `notignored` from a build
this SDK cannot read — a newer one, whose report names a word the contract of
this version does not define. The workspace binary cannot print that, and the
SDK's public entry point takes no payload, so a program that prints one is the
only way the rejection can be reached the way a user reaches it.

Its sibling in the TypeScript SDK is `test/fixtures/not-notignored.mjs`, which
covers the same branches for the same reason.

It ignores its arguments and prints `report.json`, the payload the test wrote
beside it — so what "a newer build" says lives in the test that is about it,
next to the payload every other case in the suite bends.
"""

from __future__ import annotations

import sys
from pathlib import Path

REPORT = Path(__file__).resolve().parent / "report.json"

if not REPORT.is_file():
    sys.stderr.write(f"not_notignored: no payload at {REPORT}\n")
    raise SystemExit(64)

sys.stdout.write(REPORT.read_text(encoding="utf-8"))
