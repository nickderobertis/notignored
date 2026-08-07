"""`--diff` mode, against a git repository this suite builds and commits into.

The CLI shells out to real `git` for this, so the only way to prove the SDK
forwards the flags correctly is to give it a real history to compare against.
"""

from __future__ import annotations

import asyncio
import subprocess
from pathlib import Path

import pytest

from notignored_sdk import Tool, ascan, scan

ADDED = "y = 2  # noqa: E501  # added by this change\n"


def git(repo: Path, *args: str) -> None:
    subprocess.run(["git", *args], cwd=repo, check=True, capture_output=True)


def test_diff_reports_only_the_suppressions_this_change_added(git_repo: Path) -> None:
    """The base commit's `# type: ignore` is already there; only the new one is new."""
    (git_repo / "app.py").write_text(
        (git_repo / "app.py").read_text(encoding="utf-8") + ADDED, encoding="utf-8"
    )

    whole_tree = scan(cwd=git_repo)
    changed = scan(diff=True, cwd=git_repo)

    assert sorted(directive.tool for directive in whole_tree.ignores) == sorted(
        [Tool.MYPY, Tool.RUFF]
    )
    assert [(directive.tool, directive.line) for directive in changed.ignores] == [(Tool.RUFF, 2)]


def test_diff_against_a_base_branch_reports_what_the_branch_added(git_repo: Path) -> None:
    """A named base uses merge-base semantics, so main's later commits are not ours."""
    git(git_repo, "switch", "--create", "feature")
    (git_repo / "feature.py").write_text(ADDED, encoding="utf-8")
    git(git_repo, "add", "-A")
    git(git_repo, "commit", "-m", "add a suppression")

    changed = scan(diff=True, diff_base="main", cwd=git_repo)

    assert [directive.path for directive in changed.ignores] == ["feature.py"]


def test_diff_finds_nothing_when_the_change_added_no_suppression(git_repo: Path) -> None:
    (git_repo / "app.py").write_text(
        (git_repo / "app.py").read_text(encoding="utf-8") + "z = 3\n", encoding="utf-8"
    )

    assert scan(diff=True, cwd=git_repo).ignores == ()


def test_the_async_form_diffs_the_same_way(git_repo: Path) -> None:
    (git_repo / "app.py").write_text(
        (git_repo / "app.py").read_text(encoding="utf-8") + ADDED, encoding="utf-8"
    )

    assert asyncio.run(ascan(diff=True, cwd=git_repo)) == scan(diff=True, cwd=git_repo)


def test_a_base_with_nothing_to_compare_is_rejected_before_a_process_starts(git_repo: Path) -> None:
    """The CLI rejects `--diff-base` without `--diff`; the SDK says so first."""
    with pytest.raises(ValueError, match="diff=True"):
        scan(diff_base="main", cwd=git_repo)

    with pytest.raises(ValueError, match="diff=True"):
        asyncio.run(ascan(diff_base="main", cwd=git_repo))
