#!/usr/bin/env python3
"""Package one compiled wheel payload into the release's other install forms.

Maturin owns wheel layout and metadata. The archive and npm package reuse its
binary bytes, and an artifact manifest records hashes across all three forms.
No registry or forge writes happen here.
"""

import argparse
import re
from dataclasses import asdict, dataclass, fields
from enum import StrEnum
import hashlib
import json
import shutil
import struct
import subprocess
import sys
import tarfile
import tomllib
import zipfile
from email.parser import BytesParser
from pathlib import Path
from typing import NoReturn, TypeAlias

ROOT = Path(__file__).resolve().parent.parent


def command(args: list[str], cwd: Path = ROOT) -> str:
    result = subprocess.run(args, cwd=cwd, capture_output=True, text=True, encoding="utf-8", errors="replace", check=False)
    if result.returncode:
        raise ValueError(f"{' '.join(args)} failed: {(result.stdout + result.stderr).strip()}")
    return result.stdout.strip()


class Architecture(StrEnum):
    X86_64 = "x86_64"
    AARCH64 = "aarch64"


Sha256Digest: TypeAlias = str
TargetTriple: TypeAlias = str
ReleaseVersion: TypeAlias = str
ReleaseTag: TypeAlias = str


@dataclass(frozen=True)
class PayloadHashes:
    archive: Sha256Digest
    wheel: Sha256Digest
    npm: Sha256Digest


@dataclass(frozen=True)
class ArtifactManifest:
    target: TargetTriple
    version: ReleaseVersion
    tag: ReleaseTag
    architecture: Architecture
    hashes: PayloadHashes
    archive: str
    wheel: str
    npm: str


def architecture(binary: bytes) -> Architecture:
    try:
        match binary[:4]:
            case b"\x7fELF":
                machine = struct.unpack_from("<H", binary, 18)[0]
                known = {62: Architecture.X86_64, 183: Architecture.AARCH64}
            case b"\xcf\xfa\xed\xfe":
                machine = struct.unpack_from("<I", binary, 4)[0]
                known = {0x01000007: Architecture.X86_64, 0x0100000C: Architecture.AARCH64}
            case _:
                if binary[:2] != b"MZ":
                    raise ValueError("unrecognized executable format")
                offset = struct.unpack_from("<I", binary, 60)[0]
                if binary[offset : offset + 4] != b"PE\x00\x00":
                    raise ValueError("invalid PE signature")
                machine = struct.unpack_from("<H", binary, offset + 4)[0]
                known = {0x8664: Architecture.X86_64}
    except struct.error:
        raise ValueError("truncated executable header") from None
    if machine not in known:
        raise ValueError(f"unsupported executable machine {machine:#x}")
    return known[machine]


def cargo_version() -> ReleaseVersion:
    """The crate version, checked against the repository's one release grammar."""
    version = tomllib.loads((ROOT / "Cargo.toml").read_text(encoding="utf-8")).get("package", {}).get("version")
    if not isinstance(version, str):
        raise ValueError("Cargo.toml [package] has no version string")
    command(["node", "scripts/release-version.mjs", "validate", "--kind", "release", "--value", version])
    return version


def pack(target: str, wheel: Path, out: Path) -> None:
    workflow_targets = tomllib.loads((ROOT / "rust-toolchain.toml").read_text(encoding="utf-8"))["toolchain"]["targets"]
    if target not in workflow_targets:
        raise ValueError(f"unsupported target: {target}")
    version = cargo_version()
    dist = f"notignored_cli-{version}"
    binary_name = "notignored.exe" if "windows" in target else "notignored"
    binary_member = f"{dist}.data/scripts/{binary_name}"
    with zipfile.ZipFile(wheel) as archive:
        names = archive.namelist()
        if names.count(binary_member) != 1 or names.count(f"{dist}.dist-info/METADATA") != 1:
            raise ValueError("wheel must contain the exact CLI payload and METADATA")
        info = BytesParser().parsebytes(archive.read(f"{dist}.dist-info/METADATA"))
        if info["Name"] != "notignored-cli" or info["Version"] != version:
            raise ValueError("wheel name/version differs from Cargo.toml")
        wheel_info = BytesParser().parsebytes(archive.read(f"{dist}.dist-info/WHEEL"))
        tags = wheel_info.get_all("Tag", [])
        payload = archive.read(binary_member)
    arch = architecture(payload)
    if not target.startswith(arch + "-"):
        raise ValueError(f"wheel executable architecture {arch} differs from {target}")
    match target.split("-"):
        case [_, "apple", "darwin"]:
            platform_pattern = rf"macosx_\d+_\d+_{'arm64' if arch == Architecture.AARCH64 else 'x86_64'}"
            signature = b"\xcf\xfa\xed\xfe"
        case [_, "pc", "windows", "msvc"]:
            platform_pattern = "win_amd64"
            signature = b"MZ"
        case [_, "unknown", "linux", "gnu"]:
            platform_pattern = rf"(?:manylinux_\d+_\d+|manylinux\d+|linux)_{arch}"
            signature = b"\x7fELF"
        case _:
            raise ValueError(f"unsupported target: {target}")
    if not payload.startswith(signature):
        raise ValueError("executable operating-system format differs from target")
    if not tags or any(not re.fullmatch(rf"py3-none-{platform_pattern}", tag) for tag in tags):
        raise ValueError("wheel platform tags differ from target")
    if not wheel.name.startswith(dist + "-") or not any(wheel.name == dist + "-" + tag + ".whl" for tag in tags):
        raise ValueError("wheel filename differs from its metadata tags")
    out.mkdir(parents=True, exist_ok=True)
    wheels = out / "wheels"
    wheels.mkdir(exist_ok=True)
    shutil.copy2(wheel, wheels / wheel.name)
    binary = out / ("notignored.exe" if "windows" in target else "notignored")
    binary.write_bytes(payload)
    binary.chmod(0o755)
    stem = f"notignored-v{version}-{target}"
    assets = out / "archives"
    assets.mkdir(exist_ok=True)
    asset = assets / f"{stem}.tar.gz"
    with tarfile.open(asset, "w:gz") as archive:
        archive.add(binary, arcname=binary.name)
        for name in ("README.md", "LICENSE", "CHANGELOG.md"):
            archive.add(ROOT / name, arcname=name)
    checksum = hashlib.sha256(asset.read_bytes()).hexdigest()
    (assets / f"{stem}.sha256").write_text(f"{checksum}  {asset.name}\n", encoding="utf-8")
    package = Path(command(["node", "scripts/npm-build.mjs", "platform", "--target", target, "--binary", str(binary), "--out", str(out / "npm")]))
    npm = out / "npm-artifacts"
    npm.mkdir(exist_ok=True)
    npm_executable = shutil.which("npm")
    if npm_executable is None:
        raise ValueError("npm is not installed")
    npm_command = [npm_executable]
    if Path(npm_executable).suffix.lower() == ".cmd":
        npm_cli = Path(npm_executable).parent / "node_modules/npm/bin/npm-cli.js"
        if not npm_cli.is_file():
            raise ValueError("npm installation lacks its JavaScript CLI")
        npm_command = ["node", str(npm_cli)]
    command([*npm_command, "pack", "--pack-destination", str(npm)], package)
    tarballs = list(npm.glob("*.tgz"))
    if len(tarballs) != 1:
        raise ValueError("npm pack must produce exactly one tarball")
    tgz = tarballs[0]
    with tarfile.open(asset) as archive:
        member = archive.extractfile(binary.name)
        if member is None:
            raise ValueError("archive has no binary")
        archive_payload = member.read()
    with tarfile.open(tgz) as archive:
        member = archive.extractfile(f"package/bin/{binary.name}")
        if member is None:
            raise ValueError("npm package has no binary")
        npm_payload = member.read()
        manifest_member = archive.extractfile("package/package.json")
        if manifest_member is None:
            raise ValueError("npm package has no manifest")
        manifest = json.load(manifest_member)
        if manifest["version"] != version:
            raise ValueError("npm version differs from crate")
    hashes = PayloadHashes(
        archive=hashlib.sha256(archive_payload).hexdigest(),
        wheel=hashlib.sha256(payload).hexdigest(),
        npm=hashlib.sha256(npm_payload).hexdigest(),
    )
    if len(set(asdict(hashes).values())) != 1:
        raise ValueError("archive/wheel/npm payload hashes differ")
    manifest = ArtifactManifest(target, version, f"v{version}", arch, hashes, asset.name, wheel.name, tgz.name)
    (out / "manifest.json").write_text(json.dumps(asdict(manifest), indent=2) + "\n", encoding="utf-8")
    print(f"stage-release: {target} archive/wheel/npm payloads agree")


def verify_staged(out: Path) -> None:
    doc = json.loads((out / "manifest.json").read_text(encoding="utf-8"))
    if not isinstance(doc, dict):
        raise ValueError("staged manifest must be an object")
    expected = {field.name for field in fields(ArtifactManifest)}
    if set(doc) != expected:
        raise ValueError(f"staged manifest fields differ from {sorted(expected)}")
    raw_hashes = doc.pop("hashes")
    if (
        not isinstance(raw_hashes, dict)
        or set(raw_hashes) != {field.name for field in fields(PayloadHashes)}
        or any(not isinstance(value, str) or not re.fullmatch(r"[0-9a-f]{64}", value) for value in raw_hashes.values())
    ):
        raise ValueError("invalid staged manifest hashes")
    hashes = PayloadHashes(**raw_hashes)
    if any(not isinstance(value, str) for value in doc.values()):
        raise ValueError("staged manifest fields must be strings")
    if doc["architecture"] not in set(Architecture):
        raise ValueError(f"unknown staged manifest architecture {doc['architecture']}")
    doc["architecture"] = Architecture(doc["architecture"])
    manifest = ArtifactManifest(hashes=hashes, **doc)
    version = cargo_version()
    targets = tomllib.loads((ROOT / "rust-toolchain.toml").read_text(encoding="utf-8"))["toolchain"]["targets"]
    if manifest.target not in targets or manifest.version != version or manifest.tag != f"v{version}":
        raise ValueError("invalid staged manifest target/version/tag")
    if not manifest.target.startswith(manifest.architecture + "-"):
        raise ValueError("manifest architecture differs from target")
    for filename in (manifest.archive, manifest.wheel, manifest.npm):
        if not isinstance(filename, str) or Path(filename).name != filename:
            raise ValueError("artifact filename must be a basename")
    binary_name = "notignored.exe" if "windows" in manifest.target else "notignored"
    with tarfile.open(out / "archives" / manifest.archive) as archive:
        member = archive.extractfile(binary_name)
        if member is None:
            raise ValueError("archive binary is missing")
        archive_payload = member.read()
    with zipfile.ZipFile(out / "wheels" / manifest.wheel) as wheel:
        wheel_payload = wheel.read(f"notignored_cli-{version}.data/scripts/{binary_name}")
    with tarfile.open(out / "npm-artifacts" / manifest.npm) as npm:
        member = npm.extractfile(f"package/bin/{binary_name}")
        if member is None:
            raise ValueError("npm binary is missing")
        npm_payload = member.read()
    if architecture(archive_payload) != manifest.architecture:
        raise ValueError("manifest architecture differs from executable payload")
    actual = PayloadHashes(*(hashlib.sha256(payload).hexdigest() for payload in (archive_payload, wheel_payload, npm_payload)))
    if actual != hashes or len(set(asdict(actual).values())) != 1:
        raise ValueError("staged archive/wheel/npm payload hashes differ from manifest")
    print("stage-release: staged payload hashes verified")


class Arguments(argparse.ArgumentParser):
    """Argument errors carry the same next action as every other refusal."""

    def error(self, message: str) -> NoReturn:
        print(
            f"stage-release: {message}\nACTION: pack with --target TRIPLE --wheel PATH --out DIR, "
            "or verify with --verify --out DIR",
            file=sys.stderr,
        )
        sys.exit(2)


def main() -> None:
    parser = Arguments(description=__doc__)
    parser.add_argument("--target")
    parser.add_argument("--wheel", type=Path)
    parser.add_argument("--verify", action="store_true")
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()
    try:
        if args.verify:
            verify_staged(args.out.resolve())
        elif args.target and args.wheel:
            pack(args.target, args.wheel.resolve(), args.out.resolve())
        else:
            raise ValueError("packing requires --target and --wheel; verification requires --verify")
    except (OSError, ValueError, KeyError, struct.error, zipfile.BadZipFile, tarfile.TarError, IndexError, TypeError) as error:
        print(f"stage-release: {error}\nACTION: rebuild the target's wheel and inspect its metadata/payload", file=sys.stderr)
        sys.exit(1)


if __name__ == "__main__":
    main()
