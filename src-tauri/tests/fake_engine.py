#!/usr/bin/env python3
"""A stand-in for `python -m overnight_engine` that speaks the SPEC 3.3 protocol.

The real engine is being written in parallel and needs models and a venv;
the Rust runner only cares about the protocol, the exit code and the files
each stage leaves behind. This script fakes exactly that:

  check                      prints a SPEC 3.4 report
  run --dir D --from S ...   walks the stages from S, printing start /
                             progress / done lines with short sleeps and
                             writing the output files (raw.wav is a real,
                             one-second, silent WAV so the duration reader
                             has something to read)
  render|separate|convert|mix --dir D   one stage

Knobs, so a test can ask for trouble without changing the runner's arguments:
  --fail STAGE               emit an error at STAGE and exit 1
  D/fake-fail  (file)        same, the stage name is the file's contents
  D/fake-sleep (file)        seconds to sleep per progress step (default 0.02)
  FAKE_ENGINE_SLEEP (env)    same as fake-sleep

Anything that is not protocol goes to stderr, like the real engine must.
"""

import json
import os
import struct
import sys
import time

STAGES = ["render", "separate", "convert", "mix"]
OUTPUTS = {
    "render": ["raw.wav"],
    "separate": ["vocals.wav", "instrumental.wav"],
    "convert": ["vocals_converted.wav"],
    "mix": ["final.wav", "final.mp3", "cover.png"],
}


def emit(stage, event, message="", **extra):
    line = {"stage": stage, "event": event, "message": message, "at": int(time.time() * 1000)}
    line.update(extra)
    print(json.dumps(line), flush=True)


def write_wav(path, seconds=1.0, rate=8000):
    """A silent 16-bit mono PCM WAV with a correct header."""
    frames = int(seconds * rate)
    data = b"\x00\x00" * frames
    with open(path, "wb") as f:
        f.write(b"RIFF")
        f.write(struct.pack("<I", 36 + len(data)))
        f.write(b"WAVE")
        f.write(b"fmt ")
        f.write(struct.pack("<IHHIIHH", 16, 1, 1, rate, rate * 2, 2, 16))
        f.write(b"data")
        f.write(struct.pack("<I", len(data)))
        f.write(data)


def read_knob(song_dir, name, default=None):
    try:
        with open(os.path.join(song_dir, name)) as f:
            return f.read().strip()
    except OSError:
        return default


def run_stage(stage, song_dir, sleep, fail_at, force):
    if stage == "render" and not force and os.path.exists(os.path.join(song_dir, "raw.wav")):
        print("raw.wav exists, skipping render (no --force)", file=sys.stderr)
        emit(stage, "done", "raw.wav already there")
        return
    emit(stage, "start", "Loading fake %s" % stage)
    emit(stage, "log", "device: fake")
    print("a library printed this to stdout by mistake", flush=True)
    steps = 4
    for i in range(1, steps + 1):
        time.sleep(sleep)
        if fail_at == stage and i == 2:
            emit(stage, "error", "fake %s failed on purpose" % stage, detail="Traceback (fake)\n  boom")
            sys.exit(1)
        emit(stage, "progress", "Step %d/%d" % (i, steps), pct=int(i * 100 / steps))
    for name in OUTPUTS[stage]:
        p = os.path.join(song_dir, name)
        if name.endswith(".wav"):
            write_wav(p)
        else:
            with open(p, "wb") as f:
                f.write(b"fake " + name.encode())
    emit(stage, "done", "%s written" % ", ".join(OUTPUTS[stage]), detail="%.1f s" % (sleep * steps))
    print("stage %s finished" % stage, file=sys.stderr)


def main(argv):
    if not argv:
        print("usage: fake_engine <check|run|render|separate|convert|mix> ...", file=sys.stderr)
        return 2
    cmd, args = argv[0], argv[1:]
    if cmd == "check":
        print(json.dumps({
            "python": "%d.%d.%d" % sys.version_info[:3],
            "torch": "0.0-fake", "device": os.environ.get("OVERNIGHT_DEVICE", "cpu"),
            "acestep": True, "demucs": True, "rvc": False, "ffmpeg": "/opt/homebrew/bin/ffmpeg",
            "models": {"acestep": False, "demucs": False, "rvcAssets": False},
            "problems": ["this is the fake engine"],
        }))
        return 0

    song_dir = None
    start = "render"
    no_convert = False
    fail_at = None
    force = False
    i = 0
    while i < len(args):
        a = args[i]
        if a == "--dir":
            song_dir = args[i + 1]
            i += 2
        elif a == "--from":
            start = args[i + 1]
            i += 2
        elif a == "--fail":
            fail_at = args[i + 1]
            i += 2
        elif a == "--no-convert":
            no_convert = True
            i += 1
        elif a == "--force":
            force = True
            i += 1
        elif a.startswith("--"):
            # every other flag takes a value; the fake does not care which
            i += 2
        else:
            i += 1
    if not song_dir or not os.path.isdir(song_dir):
        emit("setup", "error", "no --dir")
        return 2
    print("env OVERNIGHT_DATA=%s OVERNIGHT_MODELS=%s OVERNIGHT_DEVICE=%s" % (
        os.environ.get("OVERNIGHT_DATA"), os.environ.get("OVERNIGHT_MODELS"), os.environ.get("OVERNIGHT_DEVICE")), file=sys.stderr)
    sleep = float(read_knob(song_dir, "fake-sleep", os.environ.get("FAKE_ENGINE_SLEEP", "0.02")))
    fail_at = fail_at or read_knob(song_dir, "fake-fail")

    if cmd in STAGES:
        stages = [cmd]
    elif cmd == "run":
        stages = STAGES[STAGES.index(start):]
    else:
        emit("setup", "error", "unknown command %s" % cmd)
        return 2
    for stage in stages:
        if stage == "convert" and no_convert:
            continue
        run_stage(stage, song_dir, sleep, fail_at, force)
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
