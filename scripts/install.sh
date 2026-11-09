#!/bin/sh
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
# Equivalent environment variables: NOTIGNORED_VERSION, NOTIGNORED_INSTALL_DIR,
# and NOTIGNORED_RELEASE_BASE_URL / NOTIGNORED_RELEASE_API_URL (point the
# download and the "latest" lookup at a mirror; the checksum is fetched from the
# same place, so only use a source you trust). GITHUB_TOKEN is sent to canonical
# GitHub only — never to a mirror, whoever runs it.
# Set GITHUB_TOKEN to lift the GitHub API rate limit when resolving "latest".
#
# A server error, timeout or dropped connection is retried with a doubling delay.
# NOTIGNORED_DOWNLOAD_ATTEMPTS (default 5), NOTIGNORED_DOWNLOAD_FIRST_DELAY (2s),
# NOTIGNORED_DOWNLOAD_MAX_DELAY (16s) and NOTIGNORED_DOWNLOAD_TIMEOUT (120s per
# attempt) tune that; a 404 or other permanent failure is never retried.
#
# Covers Linux (x86_64, arm64), Apple Silicon macOS (arm64; Intel macOS is no
# longer supported) and Windows x86_64 under a POSIX shell
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
# tests/install_contract.rs fails the build when they disagree. Keep the two
# lines below verbatim — the test reads them.
#
# That template names the asset *stem*: the upload action derives both published
# names from it by giving it an extension, the archive's and `.sha256`. The
# checksum is therefore a sibling of the archive, not a suffix on it —
# `notignored-v0.1.11-x86_64-unknown-linux-gnu.sha256`, never
# `…-unknown-linux-gnu.tar.gz.sha256`, which is what this script asked for
# through v0.1.11 and 404'd on every release it had ever cut.
# ASSET_NAME_TEMPLATE: $bin-$tag-$target
# CHECKSUM_NAME_TEMPLATE: $bin-$tag-$target.sha256
BASE_URL="${NOTIGNORED_RELEASE_BASE_URL:-https://github.com/$REPO/releases/download}"
API_URL="${NOTIGNORED_RELEASE_API_URL:-https://api.github.com}"

# A mirror is an origin the user chose, not one we trust: bound it to an http(s)
# URL so nothing else (a file path, a shell metacharacter, a `javascript:` string)
# reaches the downloader.
for override in "$BASE_URL" "$API_URL"; do
    case "$override" in
        http://*|https://*) ;;
        *) printf 'error: %s\n' "release URL must start with http:// or https:// (got $override)" >&2
           printf 'unset NOTIGNORED_RELEASE_BASE_URL / NOTIGNORED_RELEASE_API_URL to use the published release\n' >&2
           exit 1 ;;
    esac
done

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

# These reach shell arithmetic, so hold them to plain decimal: a leading zero is
# octal there, and an unbounded length overflows it.
download_setting() {
    case "$2" in
        0) [ "$3" = 0 ] || err "$1 must be at least $3 (got 0)" ;;
        ""|0*|*[!0-9]*|?????*) err "$1 must be a whole number from $3 to 9999 (got $2); unset it to use the default" ;;
    esac
}
DOWNLOAD_ATTEMPTS="${NOTIGNORED_DOWNLOAD_ATTEMPTS:-5}"
DOWNLOAD_FIRST_DELAY="${NOTIGNORED_DOWNLOAD_FIRST_DELAY:-2}"
DOWNLOAD_MAX_DELAY="${NOTIGNORED_DOWNLOAD_MAX_DELAY:-16}"
DOWNLOAD_TIMEOUT="${NOTIGNORED_DOWNLOAD_TIMEOUT:-120}"
download_setting NOTIGNORED_DOWNLOAD_ATTEMPTS "$DOWNLOAD_ATTEMPTS" 1
download_setting NOTIGNORED_DOWNLOAD_FIRST_DELAY "$DOWNLOAD_FIRST_DELAY" 0
download_setting NOTIGNORED_DOWNLOAD_MAX_DELAY "$DOWNLOAD_MAX_DELAY" 0
download_setting NOTIGNORED_DOWNLOAD_TIMEOUT "$DOWNLOAD_TIMEOUT" 1

VERSION="${NOTIGNORED_VERSION:-}"
INSTALL_DIR="${NOTIGNORED_INSTALL_DIR:-$HOME/.local/bin}"
[ -n "$INSTALL_DIR" ] || err "install directory is empty (set --to <dir> or NOTIGNORED_INSTALL_DIR)"

while [ $# -gt 0 ]; do
    case "$1" in
        --version) [ $# -ge 2 ] || err "--version needs a value; pass a release tag, e.g. --version v0.1.0"; VERSION="$2"; shift 2 ;;
        --to) [ $# -ge 2 ] || err "--to needs a value; pass a directory, e.g. --to \"\$HOME/.local/bin\""; INSTALL_DIR="$2"; shift 2 ;;
        -h|--help) usage; exit 0 ;;
        *) usage; err "unknown argument: $1" ;;
    esac
done

# Map uname output onto the Rust target triples the release workflow builds.
detect_target() {
    os="$(uname -s)" || err "cannot detect the OS (uname failed); install from source: cargo install --git https://github.com/$REPO --locked"
    arch="$(uname -m)" || err "cannot detect the architecture (uname failed); install from source: cargo install --git https://github.com/$REPO --locked"
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
    # Releases ship an Apple Silicon build only; say so rather than 404 on a
    # Darwin x86_64 asset that no longer exists.
    if [ "$suffix" = "apple-darwin" ] && [ "$cpu" = "x86_64" ]; then
        err "Intel macOS is no longer supported: releases ship an Apple Silicon (arm64) build only (use: cargo install --git https://github.com/$REPO --locked)"
    fi
    if [ "$suffix" = "pc-windows-msvc" ] && [ "$cpu" != "x86_64" ]; then
        err "no prebuilt Windows binary for $cpu (use: cargo install --git https://github.com/$REPO --locked)"
    fi
    TARGET="$cpu-$suffix"
}

# One download attempt, with no retry of its own. Returns the downloader's verdict
# and leaves FETCH_STATUS (the final HTTP status, empty if there was none),
# FETCH_DETAIL (what went wrong, for a person) and FETCH_TRANSIENT (1 when another
# attempt could succeed).
download_once() {
    # GITHUB_TOKEN exists only to lift api.github.com's rate limit, so it is sent
    # to that host and nowhere else. Attaching it to whatever BASE_URL happens to
    # be would hand the user's credential to any mirror they were talked into.
    auth=""
    case "$1" in
        https://api.github.com/*) auth="${GITHUB_TOKEN:-}" ;;
    esac
    FETCH_STATUS=""
    FETCH_DETAIL=""
    FETCH_TRANSIENT=0
    code=0
    if have curl; then
        FETCH_STATUS="$(curl -fsSL --max-time "$DOWNLOAD_TIMEOUT" -w '%{http_code}' \
            ${auth:+-H "Authorization: Bearer $auth"} "$1" -o "$2" 2>"$WORK/fetch.err")" || code=$?
        [ "$code" -ne 0 ] || return 0
        FETCH_DETAIL="$(cat "$WORK/fetch.err")"
        # Timeouts, DNS and connect failures, a reset or truncated transfer, an
        # interrupted TLS handshake or HTTP/2 stream.
        case "$code" in 6|7|16|18|28|35|52|55|56|92) FETCH_TRANSIENT=1 ;; esac
    elif have wget; then
        # -T bounds each stall rather than the whole transfer; wget has no total
        # limit, and it is only the fallback for a host without curl.
        wget -q -S -t 1 -T "$DOWNLOAD_TIMEOUT" -O "$2" \
            ${auth:+--header="Authorization: Bearer $auth"} "$1" 2>"$WORK/fetch.err" || code=$?
        [ "$code" -ne 0 ] || return 0
        FETCH_STATUS="$(sed -n 's/^ *HTTP\/[0-9.]* \([0-9][0-9]*\).*/\1/p' "$WORK/fetch.err" | sed -n '$p')"
        FETCH_DETAIL="wget exited $code"
        # GNU wget's "network failure": timeouts, refused and dropped connections.
        [ "$code" -ne 4 ] || FETCH_TRANSIENT=1
    else
        err "neither curl nor wget is available; install one and re-run the installer"
    fi
    case "$FETCH_STATUS" in
        # A 2xx here is a transfer cut off after its headers: the transport verdict stands.
        ""|000|2[0-9][0-9]) ;;
        408|429|5[0-9][0-9]) FETCH_TRANSIENT=1; FETCH_DETAIL="HTTP $FETCH_STATUS" ;;
        *) FETCH_TRANSIENT=0; FETCH_DETAIL="HTTP $FETCH_STATUS" ;;
    esac
    return 1
}

# fetch <url> <dest> <what it is> <what to do on failure>
#
# GitHub's release hosts return 5xx and drop connections often enough to fail a
# consumer's check on unrelated work, so a transient failure is retried with a
# doubling, capped delay. Anything else — a 404 above all — cannot get better by
# waiting and fails on its first attempt. The checksum goes through here too:
# retrying its download never weakens its verification, which stays single-shot.
fetch() {
    attempt=1
    delay="$DOWNLOAD_FIRST_DELAY"
    [ "$delay" -le "$DOWNLOAD_MAX_DELAY" ] || delay="$DOWNLOAD_MAX_DELAY"
    while :; do
        if download_once "$1" "$2"; then return 0; fi
        [ "$FETCH_STATUS" != 404 ] || err "$3 not found: $1 (HTTP 404); $4"
        [ "$FETCH_TRANSIENT" = 1 ] || err "cannot fetch $3 from $1 ($FETCH_DETAIL); $4"
        [ "$attempt" -lt "$DOWNLOAD_ATTEMPTS" ] \
            || err "cannot fetch $3 from $1 after $attempt attempts (last error: $FETCH_DETAIL); $4"
        say "fetching $3 failed on attempt $attempt of $DOWNLOAD_ATTEMPTS ($FETCH_DETAIL); retrying in ${delay}s"
        sleep "$delay"
        attempt=$((attempt + 1))
        delay=$((delay * 2))
        [ "$delay" -le "$DOWNLOAD_MAX_DELAY" ] || delay="$DOWNLOAD_MAX_DELAY"
    done
}

resolve_latest() {
    api="$API_URL/repos/$REPO/releases/latest"
    tmp="$WORK/latest.json"
    fetch "$api" "$tmp" "the latest release of $REPO" "re-run with --version vX.Y.Z to skip the lookup"
    VERSION="$(sed -n 's/.*"tag_name" *: *"\([^"]*\)".*/\1/p' "$tmp" | head -n1)"
    [ -n "$VERSION" ] \
        || err "no published release for $REPO yet; install from source: cargo install --git https://github.com/$REPO --locked"
}

# Exactly three numeric components — a glob alone would let `v1.2.3.4` through.
is_semver() {
    case "$1" in *[!0-9.]* | .* | *. | *..* | "") return 1 ;; esac
    IFS=. read -r major minor patch extra <<SEMVER
$1
SEMVER
    [ -n "$major" ] && [ -n "$minor" ] && [ -n "$patch" ] && [ -z "$extra" ]
}

sha256_of() {
    if have sha256sum; then sha256sum "$1" | cut -d' ' -f1
    elif have shasum; then shasum -a 256 "$1" | cut -d' ' -f1
    elif have openssl; then openssl dgst -sha256 "$1" | sed 's/.*= *//'
    else say "no SHA-256 tool found; install coreutils, shasum, or openssl"; return 1
    fi
}

WORK="$(mktemp -d)" || err "cannot create a temporary directory; free space in \$TMPDIR and re-run"
trap 'rm -rf "$WORK"' EXIT

detect_target
[ -n "$VERSION" ] || resolve_latest

# The tag becomes a URL path segment, so validate it at the boundary rather than
# trusting the flag, the env var, or the API response.
case "$VERSION" in
    v*) is_semver "${VERSION#v}" || err "invalid release tag: $VERSION (expected vX.Y.Z)" ;;
    *) err "invalid release tag: $VERSION (expected vX.Y.Z)" ;;
esac

# Both published names come off the one stem — see ASSET_NAME_TEMPLATE above.
ASSET="$BIN-$VERSION-$TARGET"
ARCHIVE="$ASSET.$EXT"
CHECKSUM="$ASSET.sha256"
fetch "$BASE_URL/$VERSION/$ARCHIVE" "$WORK/$ARCHIVE" "archive $ARCHIVE" \
    "check that release $VERSION publishes this target, or re-run the installer"
fetch "$BASE_URL/$VERSION/$CHECKSUM" "$WORK/$CHECKSUM" "the checksum for $ARCHIVE" \
    "refusing to install unverified — retry, or install from source: cargo install --git https://github.com/$REPO --locked"

expected="$(cut -d' ' -f1 < "$WORK/$CHECKSUM")"
actual="$(sha256_of "$WORK/$ARCHIVE")" \
    || err "cannot compute the SHA-256 of $ARCHIVE; refusing to install unverified — install sha256sum, shasum, or openssl, or install from source: cargo install --git https://github.com/$REPO --locked"
[ "$expected" = "$actual" ] \
    || err "checksum mismatch for $ARCHIVE (expected $expected, got $actual); refusing to install — retry the download, and report it at https://github.com/$REPO/issues if it persists"

extract_failed="cannot extract $ARCHIVE (truncated download?); re-run the installer"
case "$EXT" in
    tar.gz) tar -xzf "$WORK/$ARCHIVE" -C "$WORK" || err "$extract_failed" ;;
    zip) have unzip || err "unzip is required to extract $ARCHIVE; install unzip and re-run"
         unzip -q "$WORK/$ARCHIVE" -d "$WORK" || err "$extract_failed" ;;
esac

# Not `find | head`: in a pipeline the status is head's, so a failing find would
# be reported as "binary not found" instead of as a search failure.
matches="$(find "$WORK" -name "$BIN_FILE" -type f)" \
    || err "cannot search the extracted archive; re-run the installer, or install from source: cargo install --git https://github.com/$REPO --locked"
extracted="$(printf '%s\n' "$matches" | head -n1)"
[ -n "$extracted" ] \
    || err "$BIN_FILE not found inside $ARCHIVE; report it at https://github.com/$REPO/issues"

cannot_write="cannot write $INSTALL_DIR/$BIN_FILE; re-run with --to <a directory you can write to>"
mkdir -p "$INSTALL_DIR" || err "$cannot_write"
install -m 755 "$extracted" "$INSTALL_DIR/$BIN_FILE" 2>/dev/null \
    || { cp "$extracted" "$INSTALL_DIR/$BIN_FILE" && chmod 755 "$INSTALL_DIR/$BIN_FILE"; } \
    || err "$cannot_write"

case ":$PATH:" in
    *":$INSTALL_DIR:"*) say "notignored $VERSION installed: $INSTALL_DIR/$BIN_FILE" ;;
    *) say "notignored $VERSION installed: $INSTALL_DIR/$BIN_FILE (not on PATH)" ;;
esac
