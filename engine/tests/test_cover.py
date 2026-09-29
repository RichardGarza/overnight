"""Cover drawing: colour parsing, a real PNG, determinism from the song id."""

from PIL import Image, ImageStat

from overnight_engine import cover
from overnight_engine.cover import COLOUR_WORDS, DEFAULT_COLOURS, draw_cover, parse_colours


def test_parse_colours_default_and_words():
    assert parse_colours(None) == DEFAULT_COLOURS
    assert parse_colours("dark, grainy") == DEFAULT_COLOURS
    assert parse_colours("dark, grainy, a single cold light source, blue and amber") == (
        COLOUR_WORDS["blue"], COLOUR_WORDS["amber"])
    assert parse_colours("Crimson neon, teal shadows, more crimson") == (COLOUR_WORDS["crimson"], COLOUR_WORDS["teal"])


def test_parse_colours_single_word_gets_a_partner():
    assert parse_colours("just purple") == (COLOUR_WORDS["purple"], COLOUR_WORDS["amber"])
    assert parse_colours("amber only") == (COLOUR_WORDS["amber"], COLOUR_WORDS["blue"])


def test_font_is_bundled():
    assert cover.FONT_PATH.is_file()
    assert (cover.FONT_PATH.parent / "OFL.txt").is_file()


def test_draw_cover_writes_1024_png(tmp_path):
    out = tmp_path / "cover.png"
    draw_cover(out, title="Left On Read", artist="Overnight", mood="late night drive",
               style="blue and amber", song_id="20260929-111500-t3st")
    assert out.is_file()
    with Image.open(out) as im:
        assert im.format == "PNG"
        assert im.size == (1024, 1024)
        assert im.mode == "RGB"
        # Dark picture with something drawn on it: not flat black, not bright.
        mean = sum(ImageStat.Stat(im).mean) / 3
        assert 8 < mean < 110
    assert not list(tmp_path.glob("*.partial.png"))


def test_draw_cover_is_deterministic_per_song_id(tmp_path):
    a = tmp_path / "a.png"
    b = tmp_path / "b.png"
    c = tmp_path / "c.png"
    kw = dict(title="Same Title", artist="Overnight", mood="flex", style="red and gold")
    draw_cover(a, song_id="20260929-000000-aaaa", **kw)
    draw_cover(b, song_id="20260929-000000-aaaa", **kw)
    draw_cover(c, song_id="20260929-000000-bbbb", **kw)
    assert a.read_bytes() == b.read_bytes()
    assert a.read_bytes() != c.read_bytes()


def test_long_title_wraps_and_fits(tmp_path):
    out = tmp_path / "long.png"
    draw_cover(out, title="Every Streetlight On Lakeshore Knows My Name Tonight",
               artist="Overnight", mood="", style=None, song_id="x", size=512)
    with Image.open(out) as im:
        assert im.size == (512, 512)
