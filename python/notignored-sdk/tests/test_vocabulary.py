"""The words this SDK will read, and the words the CLI writes.

Two things are proven here, both through `scan` / `ascan` — the only surface a
consumer has — and both against a real subprocess.

*The rejection.* A `change` word this SDK does not define is refused rather than
guessed at, and the refusal reaches the caller as a typed error rather than a
dropped record. The workspace binary cannot print such a word, so the run is
driven by `newer_notignored`: a `notignored` from a build past this SDK, which
is exactly the state a user reaches by upgrading the CLI and not the SDK.

*The drift gate.* `Tool`, `Scope` and `Change` are three vocabularies restated in
three languages — Rust in `src/model.rs`, TypeScript in the npm SDK, Python here
— and nothing generates one from another. What keeps them from drifting is this:
each refusal names the words this SDK knows, and every one of those lists is held
against the crate's own, exhaustively, so a variant added on either side fails
until both sides have it. `npm/notignored-sdk/test/vocabulary.test.mjs` holds the
other side of the same gate.
"""

from __future__ import annotations

import asyncio
import os
import re
from collections.abc import Callable
from pathlib import Path
from typing import Any

import pytest
from conftest import REPO_ROOT, report_payload

from notignored_sdk import Change, NotignoredContractError, Scope, Tool, ascan, scan

# The stand-in program carries a `#!/usr/bin/env python3` line, which Windows
# does not run; the branch it reaches is the same one on every platform.
unix_only = pytest.mark.skipif(os.name == "nt", reason="the stand-in program is run by its shebang")


def crate_vocabulary(enum: str) -> list[str]:
    """Every word the crate spells for `enum`, read from its one source.

    `src/model.rs` maps each variant to the word that appears in reports, and the
    match is exhaustive — a variant added without a word does not compile — so
    these are all of them, in declaration order.
    """
    model = (REPO_ROOT / "src" / "model.rs").read_text(encoding="utf-8")
    words = re.findall(rf'^\s*{enum}::\w+ => "([^"]+)",$', model, re.MULTILINE)
    assert words, f"no {enum} words found in src/model.rs; the reader below stopped seeing them"
    return words


def refusal(newer_notignored: Callable[[dict[str, Any]], Path], directive: dict[str, Any]) -> str:
    """What a caller is told when a report names a word this SDK does not know."""
    newer_notignored(report_payload(directive=directive))
    with pytest.raises(NotignoredContractError) as raised:
        scan(cwd=REPO_ROOT)
    return str(raised.value)


@unix_only
def test_a_change_word_from_a_newer_cli_reaches_the_caller_as_a_typed_error(
    newer_notignored: Callable[[dict[str, Any]], Path],
) -> None:
    """The journey a user meets: a newer CLI, a word this SDK cannot read.

    Reported, never guessed at and never dropped — a scan that quietly returned
    a record whose `change` it had thrown away would be reporting a suppression
    as something it is not.
    """
    message = refusal(newer_notignored, {"change": "rules-widened"})

    assert "'rules-widened'" in message
    assert "which this SDK does not know" in message
    assert "upgrade notignored-sdk" in message


@unix_only
def test_a_change_that_is_not_a_word_at_all_is_refused_through_the_same_call(
    newer_notignored: Callable[[dict[str, Any]], Path],
) -> None:
    message = refusal(newer_notignored, {"change": 3})

    assert "change" in message
    assert "not a string or null" in message


@unix_only
def test_the_async_form_refuses_it_too(
    newer_notignored: Callable[[dict[str, Any]], Path],
) -> None:
    """`scan` and `ascan` are one call in two forms; a verdict on one is a bug
    until it is on both."""
    newer_notignored(report_payload(directive={"change": "rules-widened"}))

    with pytest.raises(NotignoredContractError, match="upgrade notignored-sdk"):
        asyncio.run(ascan(cwd=REPO_ROOT))


@unix_only
@pytest.mark.parametrize(
    ("enum", "field", "out_of_contract", "known"),
    [
        ("Tool", "tool", "flake8", [tool.value for tool in Tool]),
        ("Scope", "scope", "paragraph", [scope.value for scope in Scope]),
        ("Change", "change", "rules-widened", [change.value for change in Change]),
    ],
)
def test_every_vocabulary_this_sdk_knows_is_the_crates_own(
    newer_notignored: Callable[[dict[str, Any]], Path],
    enum: str,
    field: str,
    out_of_contract: str,
    known: list[str],
) -> None:
    """The drift gate: three vocabularies, two languages, no generator.

    Both directions are checked at once by comparing the sets: a word the crate
    added and this SDK has not is a report a user would be refused, and a word
    this SDK has and the crate does not is a promise nothing can keep.
    """
    assert sorted(known) == sorted(crate_vocabulary(enum)), (
        f"the SDK's {enum} vocabulary and the crate's disagree; "
        f"update _model.py to match src/model.rs"
    )

    # And the words the refusal names a user are those same words, so the
    # message a consumer reads cannot drift from what the reader accepts.
    message = refusal(newer_notignored, {field: out_of_contract})
    for word in known:
        assert word in message, f"the refusal does not name {word}: {message}"
