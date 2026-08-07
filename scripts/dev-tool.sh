#!/usr/bin/env bash
# Run one of the pinned dev tools `just bootstrap` installed, wherever it landed.
#
# `scripts/setup-python-tools.sh` writes a venv per tool (`.dev/<tool>`) and
# `scripts/setup-js.sh` an npm tree (`.dev/js`), and each puts its binary in a
# different place on Windows than on Unix. The SDK projects' Nx targets go
# through here so a project.json never has to spell one platform's path — the
# same reason `tests/e2e/support.rs` resolves these binaries rather than naming
# them.
#
# Usage: scripts/dev-tool.sh <tool> [args...]
set -euo pipefail

# The binary is resolved against the repository root; the working directory is
# left alone, so a caller can run the tool from the project whose config it must
# pick up.
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

TOOL="${1:-}"
[ -n "$TOOL" ] || {
  echo "dev-tool: name the tool to run, e.g. 'scripts/dev-tool.sh ruff check .'" >&2
  exit 2
}
shift

case "$TOOL" in
biome | eslint | tsc)
  CANDIDATES=("$ROOT/.dev/js/node_modules/.bin/$TOOL" "$ROOT/.dev/js/node_modules/.bin/$TOOL.cmd")
  INSTALLER="just setup-js"
  ;;
ruff | mypy | pyright | ty)
  CANDIDATES=("$ROOT/.dev/$TOOL/bin/$TOOL" "$ROOT/.dev/$TOOL/Scripts/$TOOL.exe")
  INSTALLER="just setup-python-tools"
  ;;
*)
  echo "dev-tool: '$TOOL' is not one of the pinned dev tools" >&2
  echo "ACTION: add it to a scripts/setup-*.sh installer first, then to the table here" >&2
  exit 2
  ;;
esac

for candidate in "${CANDIDATES[@]}"; do
  if [ -e "$candidate" ]; then
    exec "$candidate" "$@"
  fi
done

echo "dev-tool: the pinned $TOOL is not installed" >&2
echo "ACTION: run '$INSTALLER' (or 'just bootstrap')" >&2
exit 1
