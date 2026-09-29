"""Small helpers every stage needs: the song record and audio file facts."""

from __future__ import annotations

import json
from pathlib import Path
from typing import Any, Dict, Optional


def load_song(song_dir: Path) -> Dict[str, Any]:
    p = song_dir / "song.json"
    if not p.is_file():
        raise FileNotFoundError(f"No song.json in {song_dir}")
    try:
        return json.loads(p.read_text(encoding="utf-8"))
    except json.JSONDecodeError as e:
        raise ValueError(f"song.json is not valid JSON: {e}") from e


def wav_duration(path: Path) -> Optional[float]:
    """Seconds of audio in a file, via soundfile (reads only the header)."""
    try:
        import soundfile as sf

        info = sf.info(str(path))
        return float(info.frames) / float(info.samplerate)
    except Exception:  # noqa: BLE001 - a missing or odd file just has no duration
        return None


def persona_of(song: Dict[str, Any]) -> Dict[str, Any]:
    p = song.get("persona")
    return p if isinstance(p, dict) else {}
