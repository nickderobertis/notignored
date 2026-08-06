#!/bin/sh
# notignored installer.
#
# Detect the host platform, download the matching prebuilt binary, verify it
# against the SHA-256 checksum published beside it, and install it onto your
# PATH.
#
# Install the latest release:
#   curl -fsSL https://raw.githubusercontent.com/nickderobertis/notignored/main/scripts/install.sh | sh
#
# Pin a version or choose where it lands (flags win over the env vars):
#   curl -fsSL .../install.sh | sh -s -- --version v0.1.0 --to ~/.local/bin
#
# Equivalent environment variables: NOTIGNORED_VERSION, NOTIGNORED_INSTALL_DIR.
# Set GITHUB_TOKEN to lift the GitHub API rate limit when resolving "latest".
#
# Covers Linux and macOS (x86_64, arm64) and Windows x86_64 under a POSIX shell
# (Git Bash / MSYS / WSL). For native Windows PowerShell or an unpublished
# target, use `cargo install --git https://github.com/nickderobertis/notignored
# --locked`.
#
# This script never installs a binary it cannot vouch for: with no SHA-256 tool
# available, or on a checksum mismatch, it aborts rather than degrade silently.
set -eu

REPO="nickderobertis/notignored"
BIN="notignored"
BIN_FILE="$BIN"

# The release-asset naming contract, shared with the `archive:` input of
# taiki-e/upload-rust-binary-action in .github/workflows/release.yml. The two
# must spell it identically or an install path 404s the moment they drift;
# tests/install_contract.rs fails the build when they disagree. Keep the line
# below verbatim — the test reads it.
# ASSET_NAME_TEMPLATE: $bin-$tag-$target
BASE_URL="https://github.com/$REPO/releases/download"

say() { printf '%s\n' "$*" >&2; }
err() { printf 'error: %s\n' "$*" >&2; exit 1; }
have() { command -v "$1" >/dev/null 2>&1; }

usage() {
    cat >&2 <<EOF
Usage: install.sh [--version <tag>] [--to <dir>]

  --version <tag>  release tag to install (default: the latest release)
  --to <dir>       directory to install into (default: \$HOME/.local/bin)
EOF
}

VERSION="${NOTIGNORED_VERSION:-}"
INSTALL_DIR="${NOTIGNORED_INSTALL_DIR:-$HOME/.local/bin}"

while [ $# -gt 0 ]; do
    case "$1" in
        --version) [ $# -ge 2 ] || err "--version needs a value"; VERSION="$2"; shift 2 ;;
        --to) [ $# -ge 2 ] || err "--to needs a value"; INSTALL_DIR="$2"; shift 2 ;;
        -h|--help) usage; exit 0 ;;
        *) usage; err "unknown argument: $1" ;;
    esac
done

# Map uname output onto the Rust target triples the release workflow builds.
detect_target() {
    os="$(uname -s)"
    arch="$(uname -m)"
    case "$os" in
        Linux) suffix="unknown-linux-gnu"; EXT="tar.gz" ;;
        Darwin) suffix="apple-darwin"; EXT="tar.gz" ;;
        MINGW*|MSYS*|CYGWIN*|Windows_NT)
            suffix="pc-windows-msvc"; EXT="zip"; BIN_FILE="$BIN.exe" ;;
        *) err "unsupported OS: $os (use: cargo install --git https://github.com/$REPO --locked)" ;;
    esac
    case "$arch" in
        x86_64|amd64) cpu="x86_64" ;;
        arm64|aarch64) cpu="aarch64" ;;
        *) err "unsupported architecture: $arch (use: cargo install --git https://github.com/$REPO --locked)" ;;
    esac
    if [ "$suffix" = "pc-windows-msvc" ] && [ "$cpu" != "x86_64" ]; then
        err "no prebuilt Windows binary for $cpu (use: cargo install --git https://github.com/$REPO --locked)"
    fi
    TARGET="$cpu-$suffix"
}

fetch() {
    if have curl; then curl -fsSL ${GITHUB_TOKEN:+-H "Authorization: Bearer $GITHUB_TOKEN"} "$1" -o "$2"
    elif have wget; then wget -qO "$2" "$1"
    else err "neither curl nor wget is available"
    fi
}

resolve_latest() {
    api="https://api.github.com/repos/$REPO/releases/latest"
    tmp="$WORK/latest.json"
    fetch "$api" "$tmp" || err "cannot reach the GitHub API to resolve the latest release"
    VERSION="$(sed -n 's/.*"tag_name" *: *"\([^"]*\)".*/\1/p' "$tmp" | head -n1)"
    [ -n "$VERSION" ] || err "no published release found for $REPO"
}

sha256_of() {
    if have sha256sum; then sha256sum "$1" | cut -d' ' -f1
    elif have shasum; then shasum -a 256 "$1" | cut -d' ' -f1
    elif have openssl; then openssl dgst -sha256 "$1" | sed 's/.*= *//'
    else err "no SHA-256 tool found (install coreutils, shasum, or openssl); refusing to install unverified"
    fi
}

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

detect_target
[ -n "$VERSION" ] || resolve_latest

ARCHIVE="$BIN-$VERSION-$TARGET.$EXT"
say "notignored: installing $VERSION ($TARGET) into $INSTALL_DIR"
fetch "$BASE_URL/$VERSION/$ARCHIVE" "$WORK/$ARCHIVE" \
    || err "cannot download $ARCHIVE — check that release $VERSION publishes this target"
fetch "$BASE_URL/$VERSION/$ARCHIVE.sha256" "$WORK/$ARCHIVE.sha256" \
    || err "cannot download the checksum for $ARCHIVE; refusing to install unverified"

expected="$(cut -d' ' -f1 < "$WORK/$ARCHIVE.sha256")"
actual="$(sha256_of "$WORK/$ARCHIVE")"
[ "$expected" = "$actual" ] \
    || err "checksum mismatch for $ARCHIVE (expected $expected, got $actual); refusing to install"

case "$EXT" in
    tar.gz) tar -xzf "$WORK/$ARCHIVE" -C "$WORK" ;;
    zip) have unzip || err "unzip is required to extract $ARCHIVE"; unzip -q "$WORK/$ARCHIVE" -d "$WORK" ;;
esac

extracted="$(find "$WORK" -name "$BIN_FILE" -type f | head -n1)"
[ -n "$extracted" ] || err "$BIN_FILE not found inside $ARCHIVE"

mkdir -p "$INSTALL_DIR"
install -m 755 "$extracted" "$INSTALL_DIR/$BIN_FILE" 2>/dev/null \
    || { cp "$extracted" "$INSTALL_DIR/$BIN_FILE" && chmod 755 "$INSTALL_DIR/$BIN_FILE"; }

say "notignored: installed $INSTALL_DIR/$BIN_FILE"
case ":$PATH:" in
    *":$INSTALL_DIR:"*) ;;
    *) say "notignored: add $INSTALL_DIR to your PATH to run it by name" ;;
esac
