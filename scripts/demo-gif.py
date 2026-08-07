#!/usr/bin/env python3
"""Render the animated demo GIF of a notignored session (the README hero).

Like `scripts/screenshots.sh`, this drives the **real release `notignored`
binary** over the committed fixture tree in `screenshots/fixture/` — so every
finding, reason, and count on screen is genuine CLI output, never a mockup. The
colours are genuine too: each command is run with `--color always` and the ANSI
it emits is parsed back into styled spans, so the GIF tracks
`src/cli/render.rs`'s roles automatically rather than restating them.

A terminal session is not something that can be screen-recorded hermetically, so
instead of capturing a PTY (which would need ttyd/ffmpeg) we reconstruct the
frames a terminal would draw — the command being typed, then each finding
appearing — and render them with the same **vendored, pinned JetBrains Mono
font** (`screenshots/fonts/`, the one the SVG screenshots use). The result is
deterministic and self-contained: Pillow only, no network.

The GIF is informational, like the screenshots — it is NOT hash-gated (a GIF is
not byte-reproducible across Pillow versions), so it is regenerated on demand
with `just screenshots-gif` and committed to `docs/screenshots/demo.gif`.
"""

from __future__ import annotations

import os
import re
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

from PIL import Image, ImageDraw, ImageFont

# GitHub-dark palette, matching the SVG screenshots' window (bg #0d1117).
BG = (13, 17, 23)
BAR = (22, 27, 34)
FG = (201, 209, 217)
DIM = (139, 148, 158)
PROMPT = (63, 185, 80)
DOTS = [(255, 95, 86), (255, 189, 46), (39, 201, 63)]  # traffic-light window dots

# The SGR foreground codes `src/cli/render.rs` can emit, as the colours a
# GitHub-dark terminal shows them in. Bold (`1`) is carried by the colour alone —
# only the regular weight of the font is vendored — and dim (`2`) is its own
# muted grey, which is exactly how the reason span is meant to read.
SGR_COLORS = {
    30: (110, 118, 129),
    31: (255, 123, 114),
    32: (63, 185, 80),
    33: (210, 153, 34),
    34: (88, 166, 255),
    35: (210, 168, 255),
    36: (57, 197, 207),
    37: FG,
}
SGR_RE = re.compile(r"\x1b\[([0-9;]*)m")

FONT_SIZE = 16
PAD = 20
BAR_H = 34
CURSOR = "█"
TYPE_MS = 55  # per typed chunk
LINE_MS = 190  # per finding appearing
BEAT_MS = 900  # after a command finishes
HOLD_MS = 3200  # on the final frame


def run(binary: str, args: list[str], cwd: Path) -> str:
    """Run the real binary and return stdout + stderr, the way a terminal
    interleaves them (findings, then the summary)."""
    env = dict(os.environ)
    # `--color always` already overrides these; clearing them keeps the capture
    # independent of whatever shell it was launched from.
    env.pop("NO_COLOR", None)
    env.pop("TERM", None)
    result = subprocess.run(
        [binary, *args], cwd=cwd, env=env, capture_output=True, text=True, check=False
    )
    return result.stdout + result.stderr


def parse_ansi(text: str) -> list[list[tuple[str, tuple[int, int, int]]]]:
    """Split ANSI-coloured text into lines of `(text, rgb)` spans."""
    lines: list[list[tuple[str, tuple[int, int, int]]]] = [[]]
    color = FG
    position = 0
    for match in SGR_RE.finditer(text):
        chunk = text[position : match.start()]
        for index, piece in enumerate(chunk.split("\n")):
            if index:
                lines.append([])
            if piece:
                lines[-1].append((piece, color))
        for raw in match.group(1).split(";"):
            code = int(raw) if raw else 0
            if code == 0:
                color = FG
            elif code == 2:
                color = DIM
            elif code in SGR_COLORS:
                color = SGR_COLORS[code]
            elif code - 60 in SGR_COLORS:
                color = SGR_COLORS[code - 60]  # the bright (90–97) variants
        position = match.end()
    for index, piece in enumerate(text[position:].split("\n")):
        if index:
            lines.append([])
        if piece:
            lines[-1].append((piece, color))
    while lines and not lines[-1]:
        lines.pop()
    return lines


def scripted_repo(fixture: Path, change: Path, into: Path) -> Path:
    """Build the repository the `--diff` command runs in: the fixture committed as
    the base, with the change overlay on top as the work under review.

    The same history `scripts/screenshots.sh` builds, and for the same reason —
    `--diff` asks git what a change added, so there has to be a change.
    """
    repo = into / "review"
    shutil.copytree(fixture, repo)
    env = dict(os.environ)
    env.update(
        GIT_CONFIG_GLOBAL=os.devnull,
        GIT_CONFIG_SYSTEM=os.devnull,
        GIT_AUTHOR_NAME="notignored",
        GIT_AUTHOR_EMAIL="notignored@example.invalid",
        GIT_COMMITTER_NAME="notignored",
        GIT_COMMITTER_EMAIL="notignored@example.invalid",
    )
    for args in (
        ["-c", "init.defaultBranch=main", "init", "-q"],
        ["add", "-A"],
        ["commit", "-q", "-m", "the tree as it stood before this change"],
    ):
        subprocess.run(["git", *args], cwd=repo, env=env, check=True, capture_output=True)
    shutil.copytree(change, repo, dirs_exist_ok=True)
    return repo


def build_frames(commands: list[tuple[str, list[list]]]) -> list[tuple[list, int]]:
    """The frames of the whole session: each command typed out, then its output
    appearing a line at a time, with the scrollback kept above."""
    frames: list[tuple[list, int]] = []
    history: list[list] = []

    for index, (typed, output) in enumerate(commands):
        if index:
            history.append([])
        for taken in range(0, len(typed) + 1, 2):
            line = [("$ ", PROMPT), (typed[:taken], FG), (CURSOR, DIM)]
            frames.append((history + [line], TYPE_MS))
        entered = [("$ ", PROMPT), (typed, FG)]
        history.append(entered)
        frames.append((list(history), BEAT_MS // 3))
        for line in output:
            history.append(line)
            frames.append((list(history), LINE_MS))
        frames.append((list(history), BEAT_MS))

    frames[-1] = (frames[-1][0], HOLD_MS)
    return frames


def render_gif(frames: list[tuple[list, int]], font_path: str, out: Path, cols: int) -> None:
    font = ImageFont.truetype(font_path, FONT_SIZE)
    char_width = font.getlength("M")
    ascent, descent = font.getmetrics()
    line_height = ascent + descent + 4
    rows = max(len(lines) for lines, _ in frames)
    width = PAD * 2 + round(cols * char_width)
    height = BAR_H + PAD + rows * line_height + PAD

    def draw_frame(lines: list) -> Image.Image:
        image = Image.new("RGB", (width, height), BG)
        draw = ImageDraw.Draw(image)
        # Window chrome: a title bar with three traffic-light dots.
        draw.rectangle([0, 0, width, BAR_H], fill=BAR)
        for index, color in enumerate(DOTS):
            centre_x = PAD + index * 20
            centre_y = BAR_H // 2
            draw.ellipse(
                [centre_x - 5, centre_y - 5, centre_x + 5, centre_y + 5], fill=color
            )
        y = BAR_H + PAD
        for spans in lines:
            x = float(PAD)
            for text, color in spans:
                draw.text((x, y), text, font=font, fill=color)
                x += font.getlength(text)
            y += line_height
        return image

    # One shared adaptive palette for every frame: per-frame palettes make the GIF
    # both larger and prone to colour shimmer between frames. The last frame holds
    # every colour the session ever shows, so it is the one to build it from.
    images = [draw_frame(lines) for lines, _ in frames]
    palette = images[-1].convert("P", palette=Image.ADAPTIVE, colors=32)
    quantized = [image.quantize(palette=palette, dither=Image.NONE) for image in images]
    quantized[0].save(
        out,
        save_all=True,
        append_images=quantized[1:],
        duration=[ms for _, ms in frames],
        loop=0,
        optimize=True,
        # `disposal=1` (leave the frame in place) lets the encoder store only the
        # rectangle that changed. A session only ever *adds* — a typed character,
        # then a finding — so nothing has to be cleared, and this is what keeps
        # the hero image a couple of hundred KiB instead of a megabyte.
        disposal=1,
    )


def main() -> int:
    root = Path(__file__).resolve().parent.parent
    binary = os.environ.get("NOTIGNORED_BIN", str(root / "target/release/notignored"))
    fixture = root / "screenshots/fixture"
    change = root / "screenshots/change"
    font_path = str(root / "screenshots/fonts/JetBrainsMono-Regular.ttf")
    out = Path(os.environ.get("DEMO_GIF_OUT", str(root / "docs/screenshots/demo.gif")))

    for required in (binary, font_path, str(fixture), str(change)):
        if not Path(required).exists():
            print(f"demo-gif: missing {required}", file=sys.stderr)
            print("ACTION: run `just screenshots-gif`, which builds the binary first.", file=sys.stderr)
            return 1

    with tempfile.TemporaryDirectory(prefix="notignored-gif-") as scratch:
        repo = scripted_repo(fixture, change, Path(scratch))
        scan = run(binary, ["src/", "--color", "always"], fixture)
        review = run(
            binary,
            ["--diff", "--diff-base", "main", "--color", "always"],
            repo,
        )

    commands = [
        ("notignored src/", parse_ansi(scan)),
        ("notignored --diff --diff-base main", parse_ansi(review)),
    ]
    for typed, output in commands:
        if not output:
            print(f"demo-gif: `{typed}` produced no output", file=sys.stderr)
            return 1

    cols = max(
        len(typed) + 2
        for typed, _ in commands
    )
    cols = max(
        cols,
        max(
            sum(len(text) for text, _ in line)
            for _, output in commands
            for line in output
        ),
    )
    frames = build_frames(commands)
    out.parent.mkdir(parents=True, exist_ok=True)
    render_gif(frames, font_path, out, cols)
    print(
        f"demo-gif: wrote {out} ({len(frames)} frames, {out.stat().st_size // 1024} KiB)",
        file=sys.stderr,
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
