"""Real-model run, opt-in: OVERNIGHT_E2E=1 pytest tests/test_e2e.py -s

Renders 10 seconds at 27 steps with ACE-Step, separates with demucs, mixes
with ffmpeg, and checks every stdout line is a protocol line. Needs the real
venv (run with <venv>/bin/python -m pytest) and the real models folder; the
first run downloads the weights. Takes a few minutes on an M3 Pro.
"""

import json
import os
import shutil
import subprocess
import sys
import time
from pathlib import Path

import pytest

ENGINE_DIR = Path(__file__).resolve().parent.parent
FIXTURES = Path(__file__).resolve().parent / "fixtures"

pytestmark = pytest.mark.skipif(os.environ.get("OVERNIGHT_E2E") != "1", reason="set OVERNIGHT_E2E=1 to run the real models")


def _engine(args, env):
    t0 = time.monotonic()
    cp = subprocess.run([sys.executable, "-m", "overnight_engine", *args], capture_output=True, text=True, env=env)
    events = []
    for line in cp.stdout.splitlines():
        if not line.strip():
            continue
        # Every stdout line must be a protocol object; anything else is a leak.
        obj = json.loads(line)
        assert {"stage", "event", "message", "at"} <= set(obj), obj
        events.append(obj)
    return cp, events, time.monotonic() - t0


def test_render_separate_mix_real(tmp_path):
    song_dir = tmp_path / "20260929-111500-e2e0"
    song_dir.mkdir()
    song = json.loads((FIXTURES / "song.json").read_text())
    song["id"] = song_dir.name
    song["durationSec"] = 10
    song["steps"] = 27
    (song_dir / "song.json").write_text(json.dumps(song))

    env = dict(os.environ)
    env["PYTHONPATH"] = str(ENGINE_DIR) + os.pathsep + env.get("PYTHONPATH", "")
    env["PYTHONUNBUFFERED"] = "1"
    env.setdefault("OVERNIGHT_DEVICE", "mps")
    env.setdefault("PYTORCH_ENABLE_MPS_FALLBACK", "1")

    cp, events, secs = _engine(["render", "--dir", str(song_dir)], env)
    assert cp.returncode == 0, cp.stderr[-3000:]
    assert events[0]["event"] == "start" and events[-1]["event"] == "done"
    assert any(e["event"] == "progress" and "Step" in e["message"] for e in events)
    assert (song_dir / "raw.wav").stat().st_size > 100_000
    print(f"\nrender: {secs:.1f} s")

    # Resumable: a second render keeps raw.wav and finishes at once.
    cp, events, secs = _engine(["render", "--dir", str(song_dir)], env)
    assert cp.returncode == 0 and events[-1]["event"] == "done" and secs < 30

    cp, events, secs = _engine(["separate", "--dir", str(song_dir)], env)
    assert cp.returncode == 0, cp.stderr[-3000:]
    assert (song_dir / "vocals.wav").is_file() and (song_dir / "instrumental.wav").is_file()
    print(f"separate: {secs:.1f} s")

    cp, events, secs = _engine(["mix", "--dir", str(song_dir), "--vocals-gain-db", "1.0"], env)
    assert cp.returncode == 0, cp.stderr[-3000:]
    for name in ("final.wav", "final.mp3", "cover.png"):
        assert (song_dir / name).is_file(), name
    print(f"mix: {secs:.1f} s")

    # convert against a model that does not exist: one clear error line, non-zero exit.
    cp, events, secs = _engine(["convert", "--dir", str(song_dir), "--pth", str(tmp_path / "nope.pth")], env)
    assert cp.returncode != 0
    errors = [e for e in events if e["event"] == "error"]
    assert len(errors) == 1 and "not found" in errors[0]["message"]
