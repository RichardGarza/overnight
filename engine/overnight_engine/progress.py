"""The progress protocol: one JSON object per line on stdout (SPEC 3.3).

    {"stage":"render","event":"progress","pct":42,"message":"Step 25/60","at":1727650000123}

Only these lines may ever reach the app's stdout pipe. The libraries we drive
(torch, transformers, demucs, tqdm, loguru, gradio's import-time chatter) all
print freely, so before any of them is imported we move file descriptor 1 to
a private handle for the protocol and point fd 1 at stderr. After that a
`print()` anywhere, in C code, or in a child process, lands in engine.log and
never in the protocol stream. See `capture_library_output`.
"""

from __future__ import annotations

import json
import os
import sys
import time
from typing import IO, Optional

STAGES = ("render", "separate", "convert", "mix", "setup")
EVENTS = ("start", "progress", "done", "error", "log")

# The stream protocol lines are written to. Set once by capture_library_output;
# until then (tests, ad-hoc use) it is whatever sys.stdout is right now.
_protocol: Optional[IO[str]] = None


def now_ms() -> int:
    return int(time.time() * 1000)


def capture_library_output() -> IO[str]:
    """Reserve stdout for the protocol; send everything else to stderr.

    Idempotent. Returns the protocol stream. Works at the file-descriptor
    level so it also covers native code and subprocesses that inherit fd 1.
    """
    global _protocol
    if _protocol is not None:
        return _protocol
    try:
        keep = os.dup(1)
        os.dup2(2, 1)
        _protocol = os.fdopen(keep, "w", buffering=1, encoding="utf-8", errors="replace")
    except OSError:
        # No usable fd 1 (odd embedding). Fall back to the Python-level stream.
        _protocol = sys.__stdout__ or sys.stdout
    # Python-level too, so buffered writes from `print` go the same way.
    sys.stdout = sys.stderr
    return _protocol


def protocol_stream() -> IO[str]:
    return _protocol if _protocol is not None else sys.stdout


def format_event(
    stage: str,
    event: str,
    message: str,
    pct: Optional[float] = None,
    detail: Optional[str] = None,
    at: Optional[int] = None,
) -> str:
    """Build one protocol line (without the newline). Pure, for tests."""
    if stage not in STAGES:
        raise ValueError(f"unknown stage {stage!r}")
    if event not in EVENTS:
        raise ValueError(f"unknown event {event!r}")
    obj: dict = {"stage": stage, "event": event}
    if pct is not None:
        obj["pct"] = int(max(0, min(100, round(pct))))
    obj["message"] = str(message)
    if detail:
        obj["detail"] = str(detail)
    obj["at"] = now_ms() if at is None else int(at)
    # Compact separators keep lines short; ensure_ascii off so titles with
    # accents survive. Newlines inside strings are escaped by json anyway.
    return json.dumps(obj, ensure_ascii=False, separators=(",", ":"))


class Emitter:
    """Progress events for one stage.

        p = Emitter("render")
        p.start("Loading ACE-Step")
        p.progress(42, "Step 25/60")
        p.done("raw.wav 30.0 s", detail="38.1 s")
    """

    def __init__(self, stage: str, stream: Optional[IO[str]] = None):
        if stage not in STAGES:
            raise ValueError(f"unknown stage {stage!r}")
        self.stage = stage
        self._stream = stream
        self._t0 = time.monotonic()
        self._last_pct = -1

    @property
    def stream(self) -> IO[str]:
        return self._stream if self._stream is not None else protocol_stream()

    def elapsed(self) -> float:
        return time.monotonic() - self._t0

    def emit(self, event: str, message: str, pct=None, detail=None) -> None:
        line = format_event(self.stage, event, message, pct=pct, detail=detail)
        s = self.stream
        s.write(line + "\n")
        s.flush()

    def start(self, message: str) -> None:
        self._t0 = time.monotonic()
        self.emit("start", message)

    def progress(self, pct: float, message: str) -> None:
        # Collapse repeats: a 60-step loop with a per-step callback should not
        # spam identical percentages, and never go backwards within a stage.
        p = int(max(0, min(100, round(pct))))
        if p == self._last_pct and message == getattr(self, "_last_msg", None):
            return
        self._last_pct = p
        self._last_msg = message
        self.emit("progress", message, pct=p)

    def log(self, message: str) -> None:
        self.emit("log", message)

    def done(self, message: str, detail: Optional[str] = None) -> None:
        if detail is None:
            detail = f"{self.elapsed():.1f} s"
        self.emit("done", message, detail=detail)

    def error(self, message: str, detail: Optional[str] = None) -> None:
        self.emit("error", message, detail=detail)


def stderr_note(message: str) -> None:
    """A plain line for engine.log (not the protocol)."""
    sys.stderr.write(message.rstrip("\n") + "\n")
    sys.stderr.flush()
