"""`convert`: vocals.wav -> vocals_converted.wav through RVC (SPEC 3.1).

rvc-python lives in its own venv (`<venv>-rvc`, see setup.sh) because its
pins (numpy 1.23, fairseq 0.12) cannot coexist with ACE-Step's. So this
stage is a small supervisor: it validates the arguments, spawns
`<venv>-rvc/bin/python -m overnight_engine.rvc_worker`, and relays the
worker's progress lines into our protocol stream. The worker never emits
start/done/error itself; a failure comes back as one error line that we
turn into this stage's single error event.
"""

from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
import time
from pathlib import Path
from typing import List, Optional

from . import paths
from .cli import StageError
from .progress import Emitter, protocol_stream, stderr_note
from .songs import wav_duration


class ConvertError(StageError):
    """Reported as-is: one line for the UI, the worker's detail if any."""


def worker_command(python: Path, src: Path, dst: Path, args: argparse.Namespace) -> List[str]:
    cmd = [
        str(python), "-m", "overnight_engine.rvc_worker",
        "--in", str(src), "--out", str(dst), "--pth", str(args.pth),
        "--pitch", str(int(getattr(args, "pitch", 0) or 0)),
        "--method", str(getattr(args, "method", "rmvpe") or "rmvpe"),
        "--index-rate", str(float(getattr(args, "index_rate", 0.66))),
        "--protect", str(float(getattr(args, "protect", 0.33))),
        "--filter-radius", str(int(getattr(args, "filter_radius", 3))),
        "--rms-mix-rate", str(float(getattr(args, "rms_mix_rate", 0.25))),
    ]
    if getattr(args, "index", ""):
        cmd += ["--index", str(args.index)]
    return cmd


def run(song_dir: Path, args: argparse.Namespace, em: Emitter) -> None:
    src = song_dir / "vocals.wav"
    dst = song_dir / "vocals_converted.wav"
    pth = str(getattr(args, "pth", "") or "").strip()
    index = str(getattr(args, "index", "") or "").strip()

    # Cheap checks first, so the common mistakes fail in a millisecond with
    # one clear line instead of after a 20 second torch import.
    if not pth:
        raise ConvertError("No voice model given (--pth); pick a .pth in Setup")
    if not Path(pth).is_file():
        raise ConvertError(f"Voice model not found: {pth}")
    if index and not Path(index).is_file():
        raise ConvertError(f"Voice index not found: {index}")
    if not src.is_file():
        raise ConvertError("No vocals.wav to convert; run separate first")
    python = paths.rvc_python_bin()
    if python is None:
        raise ConvertError("rvc-python is not installed; run Install engine in Setup")

    em.start(f"Converting vocal with {Path(pth).name}")
    t0 = time.monotonic()
    tmp = song_dir / "vocals_converted.partial.wav"
    tmp.unlink(missing_ok=True)

    cmd = worker_command(python, src, tmp, args)
    stderr_note("$ " + " ".join(cmd))
    env = dict(os.environ)
    env["PYTHONUNBUFFERED"] = "1"
    # The worker's stdout is a pipe we filter; its stderr is our stderr, so
    # everything it prints still ends up in engine.log.
    proc = subprocess.Popen(
        cmd, stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=sys.stderr, env=env, text=True,
    )
    error: Optional[ConvertError] = None
    out = protocol_stream()
    assert proc.stdout is not None
    for line in proc.stdout:
        line = line.rstrip("\n")
        ev = _protocol_event(line)
        if ev is None:
            stderr_note(line)
            continue
        if ev.get("event") == "error":
            error = ConvertError(str(ev.get("message") or "Voice conversion failed"), ev.get("detail"))
            continue
        if ev.get("event") in ("progress", "log"):
            out.write(line + "\n")
            out.flush()
        # start/done from the worker are not expected; drop them if they appear.
    code = proc.wait()
    if error is not None:
        raise error
    if code != 0:
        raise ConvertError(f"Voice conversion failed (worker exit {code}); see engine.log")
    if not tmp.is_file():
        raise ConvertError("Voice conversion produced no output; see engine.log")
    tmp.replace(dst)
    dur = wav_duration(dst) or 0.0
    em.done(f"vocals_converted.wav {dur:.1f} s", detail=f"{time.monotonic() - t0:.1f} s")


def _protocol_event(line: str) -> Optional[dict]:
    s = line.strip()
    if not s.startswith("{"):
        return None
    try:
        o = json.loads(s)
    except json.JSONDecodeError:
        return None
    if isinstance(o, dict) and "stage" in o and "event" in o:
        return o
    return None
