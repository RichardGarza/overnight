"""`mix`: stems -> final.wav + final.mp3 + cover.png with ffmpeg (SPEC 3.1).

Vocal (converted if present, else the model's) over the instrumental, or
raw.wav mastered as is when there are no stems. Loudness is normalised with
ffmpeg's loudnorm in two passes: the first measures, the second applies the
measured values in linear mode, which is the only way loudnorm hits the
target (-14 LUFS, -1 dBTP) without pumping. Then mp3 320k with ID3 tags and
the cover as the front picture.

The ffmpeg command lines are built by pure functions so tests can check them
without running ffmpeg.
"""

from __future__ import annotations

import argparse
import json
import re
import subprocess
import time
from pathlib import Path
from typing import Any, Dict, List, Optional

from . import cover, paths
from .progress import Emitter, stderr_note
from .songs import load_song, persona_of, wav_duration

TARGET_I = -14.0
TARGET_TP = -1.0
TARGET_LRA = 11.0
OUT_RATE = 48000
ALBUM = "Overnight"


# ------------------------------------------------------------ command lines

def loudnorm_filter(measured: Optional[Dict[str, Any]] = None) -> str:
    base = f"loudnorm=I={TARGET_I:g}:TP={TARGET_TP:g}:LRA={TARGET_LRA:g}"
    if measured is None:
        return base + ":print_format=json"
    return (
        base
        + f":measured_I={measured['input_i']}:measured_TP={measured['input_tp']}"
        + f":measured_LRA={measured['input_lra']}:measured_thresh={measured['input_thresh']}"
        + f":offset={measured['target_offset']}:linear=true:print_format=summary"
    )


def build_mix_command(ffmpeg: str, vocals: Path, instrumental: Path, gain_db: float, out: Path) -> List[str]:
    """Vocal over beat into a float wav (no clipping before loudnorm).
    Both inputs are resampled to OUT_RATE first because demucs writes 44.1k
    and RVC whatever its model was trained at, and amix wants one rate."""
    graph = (
        f"[0:a]aresample={OUT_RATE},volume={gain_db:g}dB[v];"
        f"[1:a]aresample={OUT_RATE}[i];"
        f"[v][i]amix=inputs=2:duration=longest:dropout_transition=0:normalize=0[m]"
    )
    return [
        ffmpeg, "-y", "-hide_banner", "-nostdin", "-loglevel", "error",
        "-i", str(vocals), "-i", str(instrumental),
        "-filter_complex", graph, "-map", "[m]",
        "-c:a", "pcm_f32le", "-ar", str(OUT_RATE), str(out),
    ]


def build_measure_command(ffmpeg: str, src: Path) -> List[str]:
    return [
        ffmpeg, "-hide_banner", "-nostdin", "-nostats", "-i", str(src),
        "-af", loudnorm_filter(None), "-f", "null", "-",
    ]


def build_master_command(ffmpeg: str, src: Path, measured: Optional[Dict[str, Any]], out: Path) -> List[str]:
    return [
        ffmpeg, "-y", "-hide_banner", "-nostdin", "-loglevel", "error", "-i", str(src),
        "-af", loudnorm_filter(measured), "-ar", str(OUT_RATE), "-c:a", "pcm_s16le", str(out),
    ]


def build_mp3_command(
    ffmpeg: str, final_wav: Path, cover_png: Optional[Path], tags: Dict[str, str], bitrate: int, out: Path
) -> List[str]:
    cmd = [ffmpeg, "-y", "-hide_banner", "-nostdin", "-loglevel", "error", "-i", str(final_wav)]
    if cover_png is not None:
        cmd += ["-i", str(cover_png)]
    cmd += ["-map", "0:a"]
    if cover_png is not None:
        # The picture is stored as a JPEG frame; a grainy 1024px PNG would be
        # a couple of MB in every file for no visible gain.
        cmd += ["-map", "1:v", "-c:v", "mjpeg", "-q:v", "3", "-disposition:v", "attached_pic",
                "-metadata:s:v", "title=Album cover", "-metadata:s:v", "comment=Cover (front)"]
    cmd += ["-c:a", "libmp3lame", "-b:a", f"{int(bitrate)}k", "-id3v2_version", "3"]
    for k, v in tags.items():
        if v:
            cmd += ["-metadata", f"{k}={v}"]
    cmd.append(str(out))
    return cmd


def parse_loudnorm_json(stderr_text: str) -> Optional[Dict[str, Any]]:
    """loudnorm prints its measurement as the last JSON object on stderr."""
    m = None
    for m in re.finditer(r"\{[^{}]*\"input_i\"[^{}]*\}", stderr_text, re.S):
        pass
    if m is None:
        return None
    try:
        d = json.loads(m.group(0))
    except json.JSONDecodeError:
        return None
    needed = ("input_i", "input_tp", "input_lra", "input_thresh", "target_offset")
    if not all(k in d for k in needed):
        return None
    # ffmpeg prints "-inf" for silence; loudnorm cannot take that back as a
    # measured value, so treat it as "measure failed" and use dynamic mode.
    if any("inf" in str(d[k]) or "nan" in str(d[k]).lower() for k in needed):
        return None
    return d


def id3_tags(song: Dict[str, Any]) -> Dict[str, str]:
    persona = persona_of(song)
    created = str(song.get("createdAt") or "")
    year = created[:4] if len(created) >= 4 and created[:4].isdigit() else time.strftime("%Y")
    return {
        "title": str(song.get("title") or "Untitled"),
        "artist": str(persona.get("name") or ALBUM),
        "album": ALBUM,
        "date": year,
        "genre": "Hip-Hop",
        "comment": str(song.get("theme") or ""),
    }


# ------------------------------------------------------------------ running

def _run(cmd: List[str], what: str) -> subprocess.CompletedProcess:
    stderr_note("$ " + " ".join(cmd))
    cp = subprocess.run(cmd, stdin=subprocess.DEVNULL, capture_output=True, text=True)
    if cp.stderr:
        stderr_note(cp.stderr.rstrip())
    if cp.returncode != 0:
        tail = "\n".join(cp.stderr.strip().splitlines()[-6:])
        raise RuntimeError(f"ffmpeg failed while {what}: {tail or 'exit ' + str(cp.returncode)}")
    return cp


def pick_sources(song_dir: Path):
    """(vocal, instrumental) stems, or (None, None) meaning master raw.wav."""
    instrumental = song_dir / "instrumental.wav"
    for name in ("vocals_converted.wav", "vocals.wav"):
        v = song_dir / name
        if v.is_file() and instrumental.is_file():
            return v, instrumental
    return None, None


def ensure_cover(song_dir: Path, song: Dict[str, Any]) -> Optional[Path]:
    out = song_dir / "cover.png"
    if out.is_file():
        return out
    persona = persona_of(song)
    try:
        cover.draw_cover(
            out,
            title=str(song.get("title") or "Untitled"),
            artist=str(persona.get("name") or ALBUM),
            mood=str(song.get("mood") or ""),
            style=persona.get("coverStyle"),
            song_id=str(song.get("id") or song_dir.name),
        )
        return out
    except Exception as e:  # noqa: BLE001 - a song without a cover is still a song
        stderr_note(f"cover drawing failed: {e!r}")
        return None


def run(song_dir: Path, args: argparse.Namespace, em: Emitter) -> None:
    ffmpeg = paths.find_ffmpeg()
    if not ffmpeg:
        raise FileNotFoundError("ffmpeg not found; install it with: brew install ffmpeg")
    song = load_song(song_dir)
    gain = float(getattr(args, "vocals_gain_db", 0.0) or 0.0)
    bitrate = int(getattr(args, "bitrate", 320) or 320)
    t0 = time.monotonic()

    vocals, instrumental = pick_sources(song_dir)
    raw = song_dir / "raw.wav"
    scratch = song_dir / "mix.partial.wav"
    if vocals is not None and instrumental is not None:
        em.start(f"Mixing {vocals.name} over {instrumental.name}")
        _run(build_mix_command(ffmpeg, vocals, instrumental, gain, scratch), "mixing the stems")
        src = scratch
    elif raw.is_file():
        em.start("No stems; mastering raw.wav as is")
        src = raw
    else:
        raise FileNotFoundError("Nothing to mix: no stems and no raw.wav")

    em.progress(30, "Measuring loudness")
    measured = parse_loudnorm_json(_run(build_measure_command(ffmpeg, src), "measuring loudness").stderr)
    if measured is None:
        em.log("Loudness measurement unreadable; using single-pass loudnorm")
    em.progress(50, "Normalising to -14 LUFS")
    final_tmp = song_dir / "final.partial.wav"
    _run(build_master_command(ffmpeg, src, measured, final_tmp), "normalising loudness")
    final_wav = song_dir / "final.wav"
    final_tmp.replace(final_wav)
    scratch.unlink(missing_ok=True)

    em.progress(75, "Drawing the cover")
    cover_png = ensure_cover(song_dir, song)

    em.progress(85, f"Encoding mp3 {bitrate}k")
    mp3_tmp = song_dir / "final.partial.mp3"
    _run(build_mp3_command(ffmpeg, final_wav, cover_png, id3_tags(song), bitrate, mp3_tmp), "encoding the mp3")
    mp3_tmp.replace(song_dir / "final.mp3")

    dur = wav_duration(final_wav) or 0.0
    em.done(f"final.mp3 {dur:.1f} s", detail=f"{time.monotonic() - t0:.1f} s")
