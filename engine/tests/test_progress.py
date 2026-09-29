"""The progress protocol: line format, and that library output never reaches stdout."""

import io
import json
import subprocess
import sys
import textwrap

import pytest

from overnight_engine import progress
from overnight_engine.progress import Emitter, format_event


def test_format_event_shape_and_order():
    line = format_event("render", "progress", "Step 25/60", pct=42, at=1727650000123)
    assert line == '{"stage":"render","event":"progress","pct":42,"message":"Step 25/60","at":1727650000123}'
    obj = json.loads(line)
    assert list(obj) == ["stage", "event", "pct", "message", "at"]


def test_format_event_detail_and_no_pct():
    line = format_event("separate", "error", "boom", detail="Traceback...", at=5)
    obj = json.loads(line)
    assert obj == {"stage": "separate", "event": "error", "message": "boom", "detail": "Traceback...", "at": 5}
    assert "pct" not in obj


def test_pct_is_clamped_and_rounded():
    assert json.loads(format_event("mix", "progress", "x", pct=141.7))["pct"] == 100
    assert json.loads(format_event("mix", "progress", "x", pct=-3))["pct"] == 0
    assert json.loads(format_event("mix", "progress", "x", pct=41.6))["pct"] == 42


def test_unknown_stage_or_event_rejected():
    with pytest.raises(ValueError):
        format_event("lyrics", "start", "no")
    with pytest.raises(ValueError):
        format_event("render", "finished", "no")
    with pytest.raises(ValueError):
        Emitter("cover")


def test_message_with_newlines_and_unicode_stays_one_line():
    line = format_event("render", "log", "café\nsecond line", at=1)
    assert "\n" not in line
    assert json.loads(line)["message"] == "café\nsecond line"


def test_emitter_writes_one_json_line_per_call():
    buf = io.StringIO()
    em = Emitter("render", stream=buf)
    em.start("Loading")
    em.progress(10, "Step 6/60")
    em.progress(10, "Step 6/60")  # duplicate: dropped
    em.log("Device: mps")
    em.done("raw.wav 30.0 s", detail="38.1 s")
    lines = buf.getvalue().splitlines()
    assert len(lines) == 4
    events = [json.loads(l) for l in lines]
    assert [e["event"] for e in events] == ["start", "progress", "log", "done"]
    assert all(e["stage"] == "render" for e in events)
    assert events[3]["detail"] == "38.1 s"
    assert all(isinstance(e["at"], int) and e["at"] > 1_600_000_000_000 for e in events)


def test_done_default_detail_is_elapsed_seconds():
    buf = io.StringIO()
    em = Emitter("mix", stream=buf)
    em.start("x")
    em.done("y")
    detail = json.loads(buf.getvalue().splitlines()[-1])["detail"]
    assert detail.endswith(" s")
    float(detail[:-2])


def test_library_stdout_never_reaches_protocol(tmp_path):
    """Run a child that prints junk to stdout every way it can after the
    capture is installed; only protocol lines may come back on the pipe."""
    script = textwrap.dedent(
        """
        import os, sys, subprocess
        from overnight_engine.progress import capture_library_output, Emitter
        capture_library_output()
        print("python print junk")
        sys.stdout.write("sys.stdout junk\\n")
        os.write(1, b"raw fd1 junk\\n")
        subprocess.run([sys.executable, "-c", "print('child process junk')"])
        em = Emitter("setup")
        em.log("hello")
        em.done("bye", detail="0.0 s")
        """
    )
    cp = subprocess.run(
        [sys.executable, "-c", script],
        capture_output=True, text=True,
        env={**__import__("os").environ, "PYTHONPATH": str(__import__("pathlib").Path(progress.__file__).parents[1])},
    )
    assert cp.returncode == 0, cp.stderr
    lines = cp.stdout.splitlines()
    assert len(lines) == 2, cp.stdout
    for l in lines:
        obj = json.loads(l)
        assert obj["stage"] == "setup"
    assert "junk" in cp.stderr
    assert "child process junk" in cp.stderr
