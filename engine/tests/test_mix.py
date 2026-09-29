"""mix: ffmpeg command building, source selection, loudnorm parsing, tags."""

from pathlib import Path

from overnight_engine import mix


FF = "/opt/homebrew/bin/ffmpeg"


def test_build_mix_command_resamples_gains_and_sums():
    cmd = mix.build_mix_command(FF, Path("/s/vocals.wav"), Path("/s/instrumental.wav"), -2.5, Path("/s/mix.wav"))
    assert cmd[0] == FF
    assert cmd[cmd.index("-i") + 1] == "/s/vocals.wav"
    assert "/s/instrumental.wav" in cmd
    graph = cmd[cmd.index("-filter_complex") + 1]
    assert "aresample=48000" in graph
    assert "volume=-2.5dB" in graph
    assert "amix=inputs=2" in graph and "normalize=0" in graph
    assert cmd[-1] == "/s/mix.wav"
    assert "pcm_f32le" in cmd
    assert "-y" in cmd and "-nostdin" in cmd


def test_loudnorm_filter_two_pass_values():
    assert mix.loudnorm_filter(None) == "loudnorm=I=-14:TP=-1:LRA=11:print_format=json"
    measured = {"input_i": "-23.10", "input_tp": "-4.20", "input_lra": "6.30",
                "input_thresh": "-33.50", "target_offset": "0.40"}
    f = mix.loudnorm_filter(measured)
    assert f.startswith("loudnorm=I=-14:TP=-1:LRA=11:")
    assert "measured_I=-23.10" in f and "measured_TP=-4.20" in f
    assert "measured_LRA=6.30" in f and "measured_thresh=-33.50" in f
    assert "offset=0.40" in f and "linear=true" in f


def test_master_command_targets_48k_16bit():
    cmd = mix.build_master_command(FF, Path("/s/mix.wav"), None, Path("/s/final.wav"))
    assert cmd[cmd.index("-ar") + 1] == "48000"
    assert "pcm_s16le" in cmd
    assert cmd[-1] == "/s/final.wav"


def test_mp3_command_has_tags_cover_and_bitrate():
    tags = {"title": "Left On Read", "artist": "Overnight", "album": "Overnight", "date": "2026", "comment": ""}
    cmd = mix.build_mp3_command(FF, Path("/s/final.wav"), Path("/s/cover.png"), tags, 320, Path("/s/final.mp3"))
    assert cmd[cmd.index("-b:a") + 1] == "320k"
    assert "libmp3lame" in cmd
    assert "attached_pic" in cmd
    assert "title=Left On Read" in cmd and "artist=Overnight" in cmd and "album=Overnight" in cmd
    assert "date=2026" in cmd
    assert "comment=" not in cmd  # empty tags are left out
    assert cmd[cmd.index("-id3v2_version") + 1] == "3"
    assert cmd[-1] == "/s/final.mp3"


def test_mp3_command_without_cover():
    cmd = mix.build_mp3_command(FF, Path("/s/final.wav"), None, {"title": "x"}, 192, Path("/s/final.mp3"))
    assert "attached_pic" not in cmd
    assert cmd.count("-i") == 1
    assert cmd[cmd.index("-b:a") + 1] == "192k"


def test_pick_sources_prefers_converted_then_plain_then_raw(tmp_path):
    assert mix.pick_sources(tmp_path) == (None, None)
    (tmp_path / "vocals.wav").write_bytes(b"")
    assert mix.pick_sources(tmp_path) == (None, None)  # no instrumental yet
    (tmp_path / "instrumental.wav").write_bytes(b"")
    assert mix.pick_sources(tmp_path) == (tmp_path / "vocals.wav", tmp_path / "instrumental.wav")
    (tmp_path / "vocals_converted.wav").write_bytes(b"")
    assert mix.pick_sources(tmp_path)[0] == tmp_path / "vocals_converted.wav"


FFMPEG_STDERR = """
[Parsed_loudnorm_0 @ 0x1]
{
\t"input_i" : "-19.52",
\t"input_tp" : "-3.10",
\t"input_lra" : "7.90",
\t"input_thresh" : "-29.83",
\t"output_i" : "-14.00",
\t"output_tp" : "-1.00",
\t"output_lra" : "6.20",
\t"output_thresh" : "-24.31",
\t"normalization_type" : "dynamic",
\t"target_offset" : "0.12"
}
"""


def test_parse_loudnorm_json():
    d = mix.parse_loudnorm_json(FFMPEG_STDERR)
    assert d["input_i"] == "-19.52" and d["target_offset"] == "0.12"
    assert mix.parse_loudnorm_json("no json here") is None
    assert mix.parse_loudnorm_json(FFMPEG_STDERR.replace('"-19.52"', '"-inf"')) is None


def test_id3_tags_from_song(fixture_song):
    t = mix.id3_tags(fixture_song)
    assert t["title"] == "Left On Read"
    assert t["artist"] == "Overnight"
    assert t["album"] == "Overnight"
    assert t["date"] == "2026"
