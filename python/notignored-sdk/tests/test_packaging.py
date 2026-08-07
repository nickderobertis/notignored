"""The distribution a release would publish, built here on every gate run.

Only a real Release can push to PyPI, so what a pull request *can* rehearse is
everything up to the upload: stamp the crate's version in, build the wheel, and
read back the metadata `pip` would resolve against. A version or a CLI pin that
drifted shows up here rather than on the registry.
"""

from __future__ import annotations

import re
import subprocess
import zipfile
from email.parser import Parser
from pathlib import Path

import pytest

from notignored_sdk._model import SUPPORTED_REPORT_VERSION

REPO_ROOT = Path(__file__).resolve().parents[3]
PACKER = REPO_ROOT / "scripts" / "python-sdk-build.mjs"


def cargo_version() -> str:
    """The one version source this repository has."""
    package = (REPO_ROOT / "Cargo.toml").read_text(encoding="utf-8").split("[package]", 1)[1]
    section = package.split("\n[", 1)[0]
    match = re.search(r'^\s*version\s*=\s*"([^"]+)"', section, re.MULTILINE)
    assert match, "Cargo.toml [package] declares no version"
    return match.group(1)


def pack(tmp_path: Path, *args: str) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        ["node", str(PACKER), "--out", str(tmp_path), *args],
        cwd=REPO_ROOT,
        capture_output=True,
        text=True,
        check=False,
    )


@pytest.fixture(scope="module")
def wheel(tmp_path_factory: pytest.TempPathFactory) -> Path:
    """A wheel built exactly the way release.yml builds the one it publishes."""
    out = tmp_path_factory.mktemp("packed")
    packed = pack(out)
    assert packed.returncode == 0, f"{packed.stdout}\n{packed.stderr}"
    package = Path(packed.stdout.strip())

    dist = out / "dist"
    build = subprocess.run(
        ["uv", "build", "--wheel", "--out-dir", str(dist), str(package)],
        cwd=REPO_ROOT,
        capture_output=True,
        text=True,
        check=False,
    )
    assert build.returncode == 0, f"{build.stdout}\n{build.stderr}"

    wheels = sorted(dist.glob("*.whl"))
    assert len(wheels) == 1, f"expected one wheel, got {wheels}"
    return wheels[0]


def metadata(wheel: Path) -> dict[str, list[str]]:
    """The wheel's METADATA, as `pip` would read it, keyed by field."""
    with zipfile.ZipFile(wheel) as archive:
        name = next(n for n in archive.namelist() if n.endswith(".dist-info/METADATA"))
        message = Parser().parsestr(archive.read(name).decode("utf-8"))
    fields: dict[str, list[str]] = {}
    for key, value in message.items():
        fields.setdefault(key, []).append(value)
    return fields


def test_the_published_package_carries_the_cargo_version(wheel: Path) -> None:
    fields = metadata(wheel)

    assert fields["Name"] == ["notignored-sdk"]
    assert fields["Version"] == [cargo_version()]


def test_the_published_package_pins_the_exact_cli_it_was_released_with(wheel: Path) -> None:
    """`pip install notignored-sdk` has to bring a binary that speaks this contract."""
    assert metadata(wheel)["Requires-Dist"] == [f"notignored-cli=={cargo_version()}"]


def test_the_published_package_ships_its_type_information(wheel: Path) -> None:
    """Without `py.typed`, a type checker reads every import from here as Any."""
    with zipfile.ZipFile(wheel) as archive:
        names = archive.namelist()

    assert "notignored_sdk/py.typed" in names
    assert "notignored_sdk/__init__.py" in names


def test_the_committed_manifest_holds_placeholders_and_never_a_version() -> None:
    """A real version here would be a second source that silently went stale."""
    manifest = (REPO_ROOT / "python" / "notignored-sdk" / "pyproject.toml").read_text("utf-8")

    assert 'version = "0.0.0.dev0"' in manifest
    assert 'dependencies = ["notignored-cli"]' in manifest
    assert cargo_version() != "0.0.0.dev0", "the placeholder would prove nothing"


def test_the_packer_refuses_a_version_pypi_cannot_index(tmp_path: Path) -> None:
    packed = pack(tmp_path, "--version", "not-a-version")

    assert packed.returncode == 1
    assert "is not a version PyPI can index" in packed.stderr
    assert "ACTION:" in packed.stderr


def test_the_packer_rejects_an_option_it_does_not_understand(tmp_path: Path) -> None:
    packed = pack(tmp_path, "--target", "x86_64-unknown-linux-gnu")

    assert packed.returncode == 1
    assert "unknown option --target" in packed.stderr


def test_the_sdk_reads_the_report_version_the_crate_writes() -> None:
    """One contract, two languages: the SDK's supported version is the crate's."""
    model = (REPO_ROOT / "src" / "model.rs").read_text(encoding="utf-8")
    match = re.search(r"pub const REPORT_VERSION: u32 = (\d+);", model)
    assert match, "src/model.rs no longer declares REPORT_VERSION"
    assert int(match.group(1)) == SUPPORTED_REPORT_VERSION
