"""Cover art without a model (SPEC 3.5).

A 1024x1024 PNG: dark grainy gradient in the persona's colours, one soft
light source, film grain, the title in a condensed sans, the persona name
small under it, the mood in a corner. Everything random is seeded from the
song id so a re-mix draws the same picture.

The font is Barlow Condensed Bold (SIL Open Font License, fonts/OFL.txt).
"""

from __future__ import annotations

import math
import random
import zlib
from pathlib import Path
from typing import List, Optional, Tuple

import numpy as np
from PIL import Image, ImageDraw, ImageFont

RGB = Tuple[int, int, int]

FONT_PATH = Path(__file__).parent / "fonts" / "BarlowCondensed-Bold.ttf"

# Colour words we understand in `coverStyle`. First match is the base tint,
# second is the light. Cold tones listed with cold values on purpose: the
# brief is "a single cold light source, blue and amber".
COLOUR_WORDS = {
    "blue": (40, 80, 170),
    "navy": (20, 35, 90),
    "teal": (20, 120, 120),
    "cyan": (30, 150, 180),
    "green": (40, 120, 70),
    "purple": (100, 50, 150),
    "violet": (110, 60, 170),
    "magenta": (170, 40, 130),
    "pink": (200, 90, 140),
    "red": (180, 40, 40),
    "crimson": (150, 20, 50),
    "orange": (220, 110, 30),
    "amber": (230, 150, 40),
    "gold": (210, 170, 60),
    "yellow": (220, 200, 60),
    "rust": (160, 70, 30),
    "cream": (230, 215, 180),
    "white": (235, 235, 230),
    "silver": (180, 185, 190),
    "grey": (110, 110, 115),
    "gray": (110, 110, 115),
    "black": (10, 10, 12),
}
DEFAULT_COLOURS: Tuple[RGB, RGB] = (COLOUR_WORDS["blue"], COLOUR_WORDS["amber"])
TEXT = (242, 236, 228)
DIM = (205, 198, 188)


def parse_colours(style: Optional[str]) -> Tuple[RGB, RGB]:
    """Pick up to two colour words from a free-text style description."""
    found: List[RGB] = []
    for word in "".join(c if c.isalpha() else " " for c in (style or "").lower()).split():
        c = COLOUR_WORDS.get(word)
        if c and c not in found:
            found.append(c)
        if len(found) == 2:
            break
    if not found:
        return DEFAULT_COLOURS
    if len(found) == 1:
        # One colour named: pair it with amber, or with blue if it was amber.
        other = DEFAULT_COLOURS[1] if found[0] != DEFAULT_COLOURS[1] else DEFAULT_COLOURS[0]
        return found[0], other
    return found[0], found[1]


def seed_from(song_id: str) -> int:
    return zlib.crc32((song_id or "").encode("utf-8")) & 0xFFFFFFFF


def _font(size: int) -> ImageFont.FreeTypeFont:
    return ImageFont.truetype(str(FONT_PATH), size=size)


def _background(size: int, base: RGB, light: RGB, rng: random.Random) -> Image.Image:
    """Dark gradient tinted with `base`, one soft spot of `light`, vignette, grain."""
    y, x = np.mgrid[0:size, 0:size].astype(np.float32) / float(size)

    angle = rng.uniform(0, 2 * math.pi)
    g = (x * math.cos(angle) + y * math.sin(angle) + 1.0) / 2.0  # 0..1 across the frame
    g = np.clip(g, 0, 1)
    base_arr = np.array(base, dtype=np.float32) / 255.0
    near_black = np.array([0.03, 0.03, 0.04], dtype=np.float32)
    dark_tint = near_black * 0.6 + base_arr * 0.3
    img = near_black[None, None, :] * (1 - g[..., None]) + dark_tint[None, None, :] * g[..., None]

    # One cold/warm light: a wide gaussian, high in the frame, off centre.
    lx, ly = rng.uniform(0.2, 0.8), rng.uniform(0.1, 0.45)
    radius = rng.uniform(0.28, 0.42)
    d2 = (x - lx) ** 2 + (y - ly) ** 2
    spot = np.exp(-d2 / (2 * radius * radius)).astype(np.float32)
    light_arr = np.array(light, dtype=np.float32) / 255.0
    img = img + spot[..., None] * light_arr[None, None, :] * rng.uniform(0.45, 0.7)

    # Vignette so the edges fall to black and the text sits on something quiet.
    r2 = (x - 0.5) ** 2 + (y - 0.5) ** 2
    img = img * (1.0 - 0.55 * np.clip(r2 * 2.0, 0, 1))[..., None]

    # Film grain: monochrome noise, a touch stronger in the shadows.
    noise = np.random.default_rng(rng.getrandbits(32)).normal(0.0, 0.035, (size, size)).astype(np.float32)
    img = img + noise[..., None] * (1.2 - img.mean(axis=2, keepdims=True))

    arr = (np.clip(img, 0, 1) * 255).astype(np.uint8)
    return Image.fromarray(arr, "RGB")


def _fit_title(draw: ImageDraw.ImageDraw, title: str, max_w: int, max_h: int) -> Tuple[ImageFont.FreeTypeFont, List[str]]:
    """Largest size at which the title fits the box on one to three lines."""
    words = title.split() or ["Untitled"]
    for size in range(220, 40, -8):
        font = _font(size)
        for n_lines in (1, 2, 3):
            lines = _wrap(words, n_lines)
            if lines is None:
                continue
            widths = [draw.textlength(line, font=font) for line in lines]
            line_h = size * 0.98
            if max(widths) <= max_w and line_h * len(lines) <= max_h:
                return font, lines
    return _font(40), [title[:40]]


def _wrap(words: List[str], n_lines: int) -> Optional[List[str]]:
    """Split words into n roughly even lines by character count."""
    if n_lines == 1:
        return [" ".join(words)]
    if len(words) < n_lines:
        return None
    target = sum(len(w) + 1 for w in words) / n_lines
    lines, cur, cur_len = [], [], 0
    for w in words:
        if cur and cur_len + len(w) > target and len(lines) < n_lines - 1:
            lines.append(" ".join(cur))
            cur, cur_len = [], 0
        cur.append(w)
        cur_len += len(w) + 1
    lines.append(" ".join(cur))
    return lines


def draw_cover(
    out_path: Path,
    title: str,
    artist: str,
    mood: str,
    style: Optional[str],
    song_id: str,
    size: int = 1024,
) -> Path:
    rng = random.Random(seed_from(song_id))
    base, light = parse_colours(style)
    img = _background(size, base, light, rng)
    draw = ImageDraw.Draw(img)
    margin = int(size * 0.07)

    # Mood, small caps in the top-right corner.
    mood = (mood or "").strip().upper()
    if mood:
        f = _font(int(size * 0.03))
        w = draw.textlength(mood, font=f)
        draw.text((size - margin - w, margin), mood, font=f, fill=DIM)

    # Title, large, bottom-left, with the artist name under it.
    artist = (artist or "").strip().upper()
    artist_font = _font(int(size * 0.034))
    artist_h = int(size * 0.034 * 1.3) if artist else 0
    box_h = int(size * 0.5) - artist_h
    font, lines = _fit_title(draw, (title or "Untitled").strip(), size - 2 * margin, box_h)
    line_h = int(font.size * 0.98)
    total_h = line_h * len(lines)
    y = size - margin - artist_h - total_h - int(size * 0.01)
    shadow = (0, 0, 0)
    for line in lines:
        draw.text((margin + 4, y + 4), line, font=font, fill=shadow)
        draw.text((margin, y), line, font=font, fill=TEXT)
        y += line_h
    if artist:
        # Letter-spaced by drawing each glyph; Pillow has no tracking option.
        x = margin
        y_a = size - margin - artist_h + int(size * 0.008)
        for ch in artist:
            draw.text((x, y_a), ch, font=artist_font, fill=DIM)
            x += draw.textlength(ch, font=artist_font) + size * 0.006

    out_path.parent.mkdir(parents=True, exist_ok=True)
    tmp = out_path.with_name(out_path.stem + ".partial.png")
    img.save(tmp, "PNG", optimize=False)
    tmp.replace(out_path)
    return out_path
