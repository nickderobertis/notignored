"""What a caller gets back from a real scan, synchronously and on an event loop.

Every assertion here is on the report the real binary produced over a real tree —
never on an internal, and never on a canned payload.
"""

from __future__ import annotations

import asyncio
from pathlib import Path

import pytest

from notignored_sdk import IgnoreDirective, Report, Scope, Suppressed, Tool, ascan, scan


def test_a_folder_scan_reports_every_tool_in_the_tree(notignored_binary: Path, tree: Path) -> None:
    report = scan(cwd=tree, binary=notignored_binary)

    assert report.version == 1
    assert report.errors == ()
    assert [directive.path for directive in report.ignores] == ["app.py", "lib.rs", "widget.ts"]
    assert sorted(directive.tool for directive in report.ignores) == sorted(
        [Tool.ESLINT, Tool.RUFF, Tool.RUST]
    )


def test_a_directive_arrives_as_the_whole_record(notignored_binary: Path, tree: Path) -> None:
    """The record contract, field by field, against a suppression we wrote."""
    source = (tree / "app.py").read_text(encoding="utf-8")

    report = scan(["app.py"], tools=["ruff"], cwd=tree, binary=notignored_binary)

    assert report.ignores == (
        IgnoreDirective(
            tool=Tool.RUFF,
            scope=Scope.LINE,
            rules=("E501",),
            reason="the vendor's documented endpoint",
            path="app.py",
            line=1,
            end_line=1,
            column=source.index("# noqa") + 1,
            raw="# noqa: E501  # the vendor's documented endpoint",
            suppressed=Suppressed(start_line=1, end_line=1),
        ),
    )


def test_the_records_are_frozen_so_a_caller_cannot_edit_the_report(
    notignored_binary: Path, tree: Path
) -> None:
    directive = scan(cwd=tree, tools=["ruff"], binary=notignored_binary).ignores[0]

    with pytest.raises(AttributeError):
        directive.line = 99


def test_the_async_form_returns_the_same_report(notignored_binary: Path, tree: Path) -> None:
    """`ascan` is the same call on an event loop, not a second implementation."""
    synchronous = scan(cwd=tree, binary=notignored_binary)
    asynchronous = asyncio.run(ascan(cwd=tree, binary=notignored_binary))

    assert isinstance(asynchronous, Report)
    assert asynchronous == synchronous


def test_scanning_a_single_file_narrows_the_report(notignored_binary: Path, tree: Path) -> None:
    report = scan([tree / "app.py"], binary=notignored_binary)

    assert [Path(directive.path).name for directive in report.ignores] == ["app.py"]


def test_the_tool_filter_reports_only_what_it_names(notignored_binary: Path, tree: Path) -> None:
    filtered = scan(cwd=tree, tools=[Tool.ESLINT, "rust"], binary=notignored_binary)

    assert sorted(directive.tool for directive in filtered.ignores) == sorted(
        [Tool.ESLINT, Tool.RUST]
    )


def test_a_path_that_starts_with_a_dash_is_a_path_and_not_a_flag(
    notignored_binary: Path, tmp_path: Path
) -> None:
    """Paths go after `--`, so a filename can never be read as an option."""
    (tmp_path / "-weird.py").write_text("x = 1  # noqa: E501  # named oddly\n", encoding="utf-8")

    report = scan(["-weird.py"], cwd=tmp_path, binary=notignored_binary)

    assert [directive.path for directive in report.ignores] == ["-weird.py"]


def test_a_tree_with_nothing_to_report_is_an_empty_report(
    notignored_binary: Path, tmp_path: Path
) -> None:
    (tmp_path / "clean.py").write_text("x = 1\n", encoding="utf-8")

    report = scan(cwd=tmp_path, binary=notignored_binary)

    assert report == Report(version=1, ignores=(), errors=())


def test_a_blanket_suppression_reports_no_rules(notignored_binary: Path, tmp_path: Path) -> None:
    (tmp_path / "app.py").write_text("import os  # noqa\n", encoding="utf-8")

    directive = scan(cwd=tmp_path, binary=notignored_binary).ignores[0]

    assert directive.rules == ()
    assert directive.reason is None


def test_a_file_level_suppression_reports_a_range_without_an_end(
    notignored_binary: Path, tmp_path: Path
) -> None:
    (tmp_path / "app.py").write_text("# ruff: noqa: E501\nx = 1\n", encoding="utf-8")

    directive = scan(cwd=tmp_path, binary=notignored_binary).ignores[0]

    assert directive.scope is Scope.FILE
    assert directive.suppressed == Suppressed(start_line=1, end_line=None)


def test_an_unreadable_file_is_a_report_error_not_an_exception(
    notignored_binary: Path, tmp_path: Path
) -> None:
    """The CLI exits 2 here and still prints the report; the SDK returns it.

    Dropping it for the exit code would hide the one thing `Report.errors` is
    for — a file that could not be scanned, which is never the same as a clean one.
    """
    (tmp_path / "broken.py").write_bytes(b"x\xff\n")

    report = scan(cwd=tmp_path, binary=notignored_binary)

    assert [error.path for error in report.errors] == ["broken.py"]
    assert report.ignores == ()


def test_a_tool_and_a_scope_compare_equal_to_their_wire_names(
    notignored_binary: Path, tree: Path
) -> None:
    """The enums are the strings the CLI writes, so a caller can use either."""
    directive = scan(cwd=tree, tools=["ruff"], binary=notignored_binary).ignores[0]

    assert directive.tool == "ruff"
    assert str(directive.tool) == "ruff"
    assert directive.scope == "line"
    assert str(Scope.NEXT_LINE) == "next-line"


def test_the_env_override_points_the_sdk_at_a_binary(
    notignored_binary: Path, tree: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setenv("NOTIGNORED_BIN", str(notignored_binary))

    assert scan(cwd=tree).ignores


def test_an_explicit_binary_wins_over_the_env_override(
    notignored_binary: Path, tree: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setenv("NOTIGNORED_BIN", str(tree / "there-is-no-binary-here"))

    assert scan(cwd=tree, binary=notignored_binary).ignores


def test_the_binary_is_found_on_path_when_nothing_else_says_where(
    notignored_binary: Path, tree: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.delenv("NOTIGNORED_BIN", raising=False)
    monkeypatch.setenv("PATH", str(notignored_binary.parent))

    assert scan(cwd=tree).ignores
