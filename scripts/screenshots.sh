#!/usr/bin/env bash
# Capture the terminal screenshots that screencomp gates, galleries, and posts to
# PRs (see screencomp.toml + .github/workflows/visual-docs.yml).
#
# It drives the REAL release `notignored` binary over the committed fixture tree
# in screenshots/fixture/ — the same way the e2e suite drives it — so every
# captured character is genuine CLI output, never a mockup. Each scene is
# rendered to a deterministic SVG by `freeze` using the VENDORED, pinned font
# (screenshots/fonts/JetBrainsMono-Regular.ttf), so the bytes — and therefore the
# screencomp digests — are identical on every machine and CI runner without a
# pinned container. That byte-determinism is the whole contract: change a
# format's output and that scene's SVG (and hash) changes; otherwise it does not.
#
# Scenes — one per part of the CLI surface, so the gallery documents all of it:
#   scan         the default human report over the whole fixture, colorized.
#   diff         `--diff` against a git history this script builds inside a
#                throwaway copy of the fixture: commit the base tree, lay the
#                screenshots/change/ overlay on top, and report only what the
#                change added. Colorized.
#   tool-filter  `--tool`, repeated, narrowing that same scan. Colorized.
#   json         `--format json`, the full envelope. Scoped to one fixture file
#                so a complete envelope fits in the frame rather than being cut.
#   pr-comment   `--format markdown` over the same change — the exact body the
#                GitHub Action posts, permalinks and all.
# The three human scenes force ANSI through the pipe with `--color always`;
# `json` and `pr-comment` are plain text by contract. freeze renders both the
# same way (`--language ansi`).
#
# Output (screencomp's capture contract):
#   $SHOTS_OUT/captures.json   index: {schema, shots:[{name,toggles,hash,image}]}
#   $SHOTS_OUT/<scene>.svg     one SVG per scene
# $SHOTS_OUT defaults to shots/current/<arch> (the reusable workflow exports it
# per lane). The SVGs are also copied to docs/screenshots/ (committed) for the
# README gallery.
#
# Requires `freeze` on PATH (install the pinned version with `just screenshots-tools`).
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

arch="$(uname -m)"
case "$arch" in
  x86_64 | amd64) arch="x86_64" ;;
  arm64 | aarch64) arch="arm64" ;;
esac
SHOTS_OUT="${SHOTS_OUT:-shots/current/$arch}"
font="$repo_root/screenshots/fonts/JetBrainsMono-Regular.ttf"
fixture="$repo_root/screenshots/fixture"
change="$repo_root/screenshots/change"
docs_dir="$repo_root/docs/screenshots"

# The commit the `pr-comment` scene's permalinks are pinned to. A capture cannot
# use a real one — the sha of the commit that adds a screenshot is not knowable
# while taking it — so it is a fixed literal, which is also what keeps the bytes
# stable. `tests/e2e/markdown.rs` proves the link shape against the real renderer.
permalink_sha="0123456789abcdef0123456789abcdef01234567"

if ! command -v freeze >/dev/null 2>&1; then
  echo "screenshots: 'freeze' not on PATH. Install the pinned version with:" >&2
  echo "             just screenshots-tools" >&2
  exit 1
fi

# The binary the capture drives: release, locked, exactly what a user installs.
bin="$repo_root/target/release/notignored"
if [ -z "${SCREENSHOTS_NO_BUILD:-}" ] || [ ! -x "$bin" ]; then
  cargo build --release --locked --bin notignored >&2
fi

# Portable SHA-256 (Linux coreutils vs macOS/BSD).
sha256() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | cut -d' ' -f1
  else
    shasum -a 256 "$1" | cut -d' ' -f1
  fi
}

# Deterministic freeze flags. The vendored font (embedded into the SVG as base64)
# is what makes the output reproducible across machines and lets the SVG render
# on GitHub with nothing external to fetch; everything else is fixed styling.
freeze_flags=(
  # Force terminal/ANSI mode. freeze's content-based auto-detection is flaky — it
  # intermittently misreads a colored report as a source file and then ignores
  # --font.file and hangs fetching a default font over the network. `--language
  # ansi` is unconditional and offline: it preserves the ANSI colour on the human
  # scenes and renders the plain ones verbatim.
  --language ansi
  --font.file "$font"
  --font.family "JetBrains Mono"
  --font.size 14
  --window
  --background "#0d1117"
  --padding "20,30"
  --margin 0
  --border.radius 8
  # Fixed window width + wrap column so EVERY scene renders at the SAME pixel
  # width. The gallery and the README display each SVG at one fixed width, so a
  # per-scene auto-width would make the on-page text size lurch between cards —
  # a narrow `tool-filter` scaling up huge next to a wide `pr-comment`. 102
  # columns clears the widest human line (101) with the permalinks in
  # `pr-comment` folding at the same budget; 919px = 30+30 padding + 102 columns
  # at the font's ~8.42px advance.
  --width 919
  --wrap 102
)

rm -rf "$SHOTS_OUT"
mkdir -p "$SHOTS_OUT" "$docs_dir"
tmp_state="$(mktemp -d)"
trap 'rm -rf "$tmp_state"' EXIT

# captures.json identity is `name + JSON.stringify(toggles)`; entries collect one
# "name|toggles|hash|image" record per rendered scene, sorted at the end.
entries=()

# Render one captured text file to a scene SVG, hash it, and record it. A scene
# marked `require_ansi=1` must carry ANSI escapes: freeze needs them for the
# coloured render, and their absence means the binary produced no report — which
# `--language ansi` would otherwise paper over as a blank window. Plain scenes
# only have to be non-empty.
render_scene() {
  scene_name="$1"
  scene_toggles="$2"
  scene_image="$3"
  scene_src="$4"
  scene_require_ansi="$5"
  if [ ! -s "$scene_src" ]; then
    {
      echo "screenshots: scene '$scene_name' produced no output — cannot render."
      echo "ACTION: run that scene's command by hand from screenshots/fixture/ and"
      echo "        see what it prints; an empty report usually means the fixture"
      echo "        lost the directives the scene was written around."
    } >&2
    exit 1
  fi
  if [ "$scene_require_ansi" = 1 ] && ! grep -q "$(printf '\033')" "$scene_src"; then
    {
      echo "screenshots: scene '$scene_name' produced no ANSI — cannot render the coloured report."
      echo "ACTION: check that --color always still forces colour through a pipe."
      echo "---- captured stdout ($(wc -c <"$scene_src") bytes) ----"
      cat -v "$scene_src"
    } >&2
    exit 1
  fi
  # `< /dev/null`: freeze reads stdin whenever it is not a character device (its
  # IsPipe check), so under CI's piped stdin it would ignore the file argument and
  # render empty input ("No input"). Pointing stdin at /dev/null (a char device)
  # forces it down the read-the-file path on every runner.
  freeze "$scene_src" "${freeze_flags[@]}" -o "$SHOTS_OUT/$scene_image" </dev/null >&2
  entries+=("$scene_name|$scene_toggles|$(sha256 "$SHOTS_OUT/$scene_image")|$scene_image")
  # The committed copies: same bytes, just outside the gitignored shots/ tree.
  cp "$SHOTS_OUT/$scene_image" "$docs_dir/$scene_image"
}

# --- scan: the default human report over the whole fixture -------------------
# Run from inside the fixture so every reported path is the short relative one a
# user sees, with no per-machine prefix to normalize away.
out="$tmp_state/scan.ansi"
(cd "$fixture" && "$bin" . --color always) >"$out" 2>&1 || true
render_scene "scan" "{}" "scan.svg" "$out" 1

# --- tool-filter: the same scan, narrowed to three tools ---------------------
out="$tmp_state/tool-filter.ansi"
(cd "$fixture" && "$bin" . --tool ruff --tool mypy --tool shellcheck --color always) \
  >"$out" 2>&1 || true
render_scene "tool-filter" "{}" "tool-filter.svg" "$out" 1

# --- json: the full envelope, one file wide ----------------------------------
# Scoped to a single fixture file on purpose: the envelope is what this scene
# documents, and a whole-tree capture would run off the bottom of the frame with
# the same fields repeated seven times.
out="$tmp_state/json.txt"
(cd "$fixture" && "$bin" src/api_client.py --format json) >"$out" 2>/dev/null || true
render_scene "json" "{}" "json.svg" "$out" 0

# --- the review case: a real git history, built here -------------------------
# `--diff` asks git what a change added, so the scene needs a repository. Build a
# throwaway one from the fixture: commit the base tree, then lay the
# screenshots/change/ overlay on top as the uncommitted work a reviewer is
# looking at. Bare `--diff` then compares the work tree against HEAD.
#
# The git config is neutralized (`GIT_CONFIG_GLOBAL`/`GIT_CONFIG_SYSTEM`) and the
# identity fixed, so a developer's own gitconfig — commit signing, hooks,
# autocrlf, a default branch name — cannot change what is captured.
diff_repo="$tmp_state/review"
mkdir -p "$diff_repo"
cp -R "$fixture/." "$diff_repo/"
(
  cd "$diff_repo"
  export GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_SYSTEM=/dev/null
  export GIT_AUTHOR_NAME=notignored GIT_AUTHOR_EMAIL=notignored@example.invalid
  export GIT_COMMITTER_NAME=notignored GIT_COMMITTER_EMAIL=notignored@example.invalid
  git -c init.defaultBranch=main init -q
  git add -A
  git commit -q -m "the tree as it stood before this change"
) >/dev/null 2>&1
cp -R "$change/." "$diff_repo/"

# --- diff: only the suppressions this change added ---------------------------
out="$tmp_state/diff.ansi"
(cd "$diff_repo" && "$bin" . --diff --color always) >"$out" 2>&1 || true
render_scene "diff" "{}" "diff.svg" "$out" 1

# --- pr-comment: the body the GitHub Action posts ----------------------------
out="$tmp_state/pr-comment.txt"
(cd "$diff_repo" && "$bin" . --diff --format markdown \
  --github-repo nickderobertis/notignored --github-sha "$permalink_sha") \
  >"$out" 2>/dev/null || true
render_scene "pr-comment" "{}" "pr-comment.svg" "$out" 0

# Write captures.json: shots sorted by identity, schema 1, trailing newline — the
# exact shape screencomp's classify/manifest/gallery read. Every field is safe
# ASCII (scene names, hex digests, file names), so plain printf is sound.
{
  printf '{\n  "schema": 1,\n  "shots": [\n'
  IFS='
'
  sorted=($(printf '%s\n' "${entries[@]}" | sort))
  unset IFS
  last=$((${#sorted[@]} - 1))
  for i in "${!sorted[@]}"; do
    IFS='|' read -r name toggles hash image <<<"${sorted[$i]}"
    comma=","
    [ "$i" -eq "$last" ] && comma=""
    printf '    {\n      "name": "%s",\n      "toggles": %s,\n      "hash": "%s",\n      "image": "%s"\n    }%s\n' \
      "$name" "$toggles" "$hash" "$image" "$comma"
  done
  printf '  ]\n}\n'
} >"$SHOTS_OUT/captures.json"

echo "screenshots: wrote ${#entries[@]} shots to $SHOTS_OUT and docs/screenshots/" >&2
