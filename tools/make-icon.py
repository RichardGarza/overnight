#!/usr/bin/env python3
"""make-icon.py [--out src-tauri/icons] [--font PATH] [--preview]

Draws the Overnight app icon and writes everything the Tauri bundle needs:

    32x32.png  128x128.png  128x128@2x.png  icon.png (1024)  icon.icns  icon.ico

The mark: a dark rounded square, a thin amber crescent, and a small "ON"
wordmark in a condensed sans under it. Flat colour on the mark, no gradients.

Every size is drawn fresh (supersampled, then reduced) rather than scaled down
from the 1024 image, so the ring stays a ring at 16 px instead of turning into
a grey smudge. The icon is fixed, not personalised, so this is a developer
tool: run it once, commit the outputs. The builder never calls it.

Needs Pillow (the system python3 on the dev Mac has it; otherwise
`python3 -m pip install pillow`) and, for the .icns, Apple's `iconutil`.
"""

import argparse
import os
import shutil
import struct
import subprocess
import sys
import tempfile

try:
    from PIL import Image, ImageDraw, ImageFont
except ImportError:
    sys.exit("make-icon.py needs Pillow: python3 -m pip install pillow")

# Colours: near-black with a touch of warmth (a pure #000 tile looks like a
# hole in the Dock), amber for the moon, warm off-white for the wordmark.
TILE = (20, 18, 19, 255)
AMBER = (242, 169, 59, 255)
CREAM = (243, 235, 221, 255)

# Condensed faces that ship with macOS, best first. A .ttc needs a face index.
FONT_CANDIDATES = [
    ("/System/Library/Fonts/Supplemental/DIN Condensed Bold.ttf", 0),
    ("/System/Library/Fonts/Avenir Next Condensed.ttc", 4),  # Demi Bold
    ("/System/Library/Fonts/Supplemental/Arial Narrow Bold.ttf", 0),
    ("/System/Library/Fonts/Supplemental/Impact.ttf", 0),
]


def pick_font(override):
    if override:
        return override, 0
    for path, index in FONT_CANDIDATES:
        if os.path.exists(path):
            return path, index
    sys.exit("No condensed font found. Pass one with --font /path/to/font.ttf")


def draw(size, font_path, font_index):
    """One icon at `size` px, RGBA with straight alpha."""
    # Supersample harder at small sizes: 4x is plenty at 256+, but a 16 px
    # ring needs finer coverage or its edges go blotchy.
    ss = 4 if size >= 256 else 8
    s = size * ss
    img = Image.new("RGBA", (s, s), (0, 0, 0, 0))

    # The tile follows Apple's icon grid: an 824/1024 square with ~22% corner
    # radius, so it sits in the Dock at the same visual weight as everyone else.
    inset = s * 100 / 1024
    radius = s * 185 / 1024
    ImageDraw.Draw(img).rounded_rectangle(
        [inset, inset, s - inset, s - inset], radius=radius, fill=TILE
    )

    # The crescent is an amber disc with a slightly smaller disc cut out of it,
    # pushed toward the lower right so the thick part faces up-left. Built as a
    # mask so the cut-out does not depend on the tile colour underneath.
    cx, cy = s * 0.5, s * 0.435
    r_outer = s * 0.215
    # Ring thickness (the crescent's widest point) never drops under ~1.6
    # device pixels, otherwise the smallest sizes fade to nothing.
    thickness = max(s * 0.044, 1.6 * ss)
    r_inner = r_outer - thickness * 0.5
    shift = thickness * 0.75
    mask = Image.new("L", (s, s), 0)
    md = ImageDraw.Draw(mask)
    md.ellipse([cx - r_outer, cy - r_outer, cx + r_outer, cy + r_outer], fill=255)
    md.ellipse(
        [cx + shift - r_inner, cy + shift - r_inner, cx + shift + r_inner, cy + shift + r_inner],
        fill=0,
    )
    img.paste(Image.new("RGBA", (s, s), AMBER), (0, 0), mask)

    # The wordmark. Below 40 px two letters are a smear, so leave them off and
    # let the crescent carry the icon on its own.
    if size >= 40:
        draw_wordmark(img, s, font_path, font_index, cy + r_outer)

    return img.resize((size, size), Image.Resampling.LANCZOS)


def draw_wordmark(img, s, font_path, font_index, top_y):
    """'ON', letter-spaced, centred under the crescent, sized to the tile."""
    cap_height = s * 0.115
    tracking = s * 0.018
    # Size the font by the actual ink height of a capital, not by point size:
    # DIN Condensed's metrics put its glyphs oddly high in the em box.
    probe = ImageFont.truetype(font_path, 1000, index=font_index)
    _, top, _, bottom = probe.getbbox("O")
    px = int(1000 * cap_height / (bottom - top))
    font = ImageFont.truetype(font_path, px, index=font_index)

    letters = []
    total = 0
    for ch in "ON":
        left, top, right, bottom = font.getbbox(ch)
        letters.append((ch, left, top, right - left, bottom - top))
        total += right - left
    total += tracking * (len(letters) - 1)

    x = (s - total) / 2
    baseline_gap = s * 0.075
    d = ImageDraw.Draw(img)
    for ch, left, top, w, h in letters:
        # Draw each glyph at its own ink box so tracking is visual, not metric.
        d.text((x - left, top_y + baseline_gap - top), ch, font=font, fill=CREAM)
        x += w + tracking


def png_bytes(img):
    from io import BytesIO

    buf = BytesIO()
    img.save(buf, format="PNG")
    return buf.getvalue()


def write_icns(render, out_path):
    """Apple's own tool builds the .icns from a folder of PNGs at fixed names,
    so the result is exactly what Finder and the Dock expect."""
    if not shutil.which("iconutil"):
        print("iconutil not found (not macOS?) - skipping icon.icns", file=sys.stderr)
        return False
    with tempfile.TemporaryDirectory() as tmp:
        iconset = os.path.join(tmp, "icon.iconset")
        os.mkdir(iconset)
        for base in (16, 32, 128, 256, 512):
            render(base).save(os.path.join(iconset, f"icon_{base}x{base}.png"))
            render(base * 2).save(os.path.join(iconset, f"icon_{base}x{base}@2x.png"))
        subprocess.run(["iconutil", "-c", "icns", iconset, "-o", out_path], check=True)
    return True


def write_ico(render, out_path):
    """Windows .ico: a little directory followed by one PNG per size. Written
    by hand so the sizes are our own renders, not Pillow's resizes."""
    sizes = [16, 24, 32, 48, 64, 256]
    pngs = [png_bytes(render(n)) for n in sizes]
    header = struct.pack("<HHH", 0, 1, len(sizes))
    offset = 6 + 16 * len(sizes)
    entries = b""
    for n, png in zip(sizes, pngs):
        edge = 0 if n >= 256 else n  # 0 means 256 in the ICO directory
        entries += struct.pack("<BBBBHHII", edge, edge, 0, 0, 1, 32, len(png), offset)
        offset += len(png)
    with open(out_path, "wb") as f:
        f.write(header + entries + b"".join(pngs))


def main():
    here = os.path.dirname(os.path.abspath(__file__))
    ap = argparse.ArgumentParser(description="Draw the Overnight app icon.")
    ap.add_argument("--out", default=os.path.join(here, "..", "src-tauri", "icons"))
    ap.add_argument("--font", help="a .ttf/.otf to use for the ON wordmark")
    ap.add_argument("--preview", action="store_true", help="also write preview.png (the 1024 tile on grey)")
    args = ap.parse_args()

    font_path, font_index = pick_font(args.font)
    out = os.path.abspath(args.out)
    os.makedirs(out, exist_ok=True)

    cache = {}

    def render(n):
        if n not in cache:
            cache[n] = draw(n, font_path, font_index)
        return cache[n]

    for name, n in [("32x32.png", 32), ("128x128.png", 128), ("128x128@2x.png", 256), ("icon.png", 1024)]:
        render(n).save(os.path.join(out, name))
    write_icns(render, os.path.join(out, "icon.icns"))
    write_ico(render, os.path.join(out, "icon.ico"))

    if args.preview:
        # A contact sheet on mid grey: the 1024 tile plus the Dock-ish sizes,
        # for eyeballing without installing anything.
        sheet = Image.new("RGBA", (1024 + 40 + 256 + 40, 1024), (128, 128, 128, 255))
        sheet.alpha_composite(render(1024), (0, 0))
        y = 0
        for n in (256, 128, 64, 32, 16):
            sheet.alpha_composite(render(n), (1024 + 40, y))
            y += n + 24
        sheet.save(os.path.join(out, "preview.png"))

    print(f"font: {font_path}")
    print(f"wrote: {out}")


if __name__ == "__main__":
    main()
