"""Command line entry: check | render | separate | convert | mix | run (SPEC 3.1).

The Rust backend spawns `<venv>/bin/python -m overnight_engine <cmd> --dir <songdir>`
with OVERNIGHT_DATA, OVERNIGHT_MODELS and OVERNIGHT_DEVICE in the environment.
Everything here is deliberately light: the heavy libraries are imported inside
the stage modules, only when their stage runs, so `check` and a failing
argument parse come back in well under a second.
"""

from __future__ import annotations

import argparse
import sys
import traceback
from pathlib import Path
from typing import List, Optional

from . import paths
from .progress import Emitter, capture_library_output, stderr_note

STAGE_ORDER = ["render", "separate", "convert", "mix"]


class StageError(Exception):
    """A failure with a one-line human message and optional detail."""

    def __init__(self, message: str, detail: Optional[str] = None):
        super().__init__(message)
        self.message = message
        self.detail = detail


def _add_dir(p: argparse.ArgumentParser) -> None:
    p.add_argument("--dir", required=True, help="the song folder (holds song.json)")


def _add_convert_flags(p: argparse.ArgumentParser) -> None:
    p.add_argument("--pth", default="", help="RVC model .pth (required for convert)")
    p.add_argument("--index", default="", help="RVC feature .index (optional)")
    p.add_argument("--pitch", type=int, default=0, help="transpose in semitones")
    p.add_argument("--method", default="rmvpe", choices=["rmvpe", "harvest", "crepe", "pm"])
    p.add_argument("--index-rate", type=float, default=0.66)
    p.add_argument("--protect", type=float, default=0.33)
    p.add_argument("--filter-radius", type=int, default=3)
    p.add_argument("--rms-mix-rate", type=float, default=0.25)


def _add_mix_flags(p: argparse.ArgumentParser) -> None:
    p.add_argument("--vocals-gain-db", type=float, default=0.0)
    p.add_argument("--bitrate", type=int, default=320)


def _add_render_flags(p: argparse.ArgumentParser) -> None:
    p.add_argument("--guidance", type=float, default=15.0)
    p.add_argument("--force", action="store_true", help="re-render even if raw.wav exists")


def build_parser() -> argparse.ArgumentParser:
    ap = argparse.ArgumentParser(prog="overnight_engine")
    sub = ap.add_subparsers(dest="command", required=True)

    sub.add_parser("check", help="report what is installed; never downloads")

    p = sub.add_parser("render", help="song.json -> raw.wav via ACE-Step")
    _add_dir(p)
    _add_render_flags(p)

    p = sub.add_parser("separate", help="raw.wav -> vocals.wav + instrumental.wav via demucs")
    _add_dir(p)

    p = sub.add_parser("convert", help="vocals.wav -> vocals_converted.wav via RVC")
    _add_dir(p)
    _add_convert_flags(p)

    p = sub.add_parser("mix", help="stems -> final.wav + final.mp3 + cover.png")
    _add_dir(p)
    _add_mix_flags(p)

    p = sub.add_parser("run", help="the stages in order")
    _add_dir(p)
    p.add_argument("--from", dest="from_stage", default="render", choices=STAGE_ORDER)
    p.add_argument("--no-convert", action="store_true")
    _add_render_flags(p)
    _add_convert_flags(p)
    _add_mix_flags(p)
    return ap


def _song_dir(args: argparse.Namespace) -> Path:
    d = Path(args.dir).expanduser().resolve()
    if not d.is_dir():
        raise StageError(f"Song folder not found: {d}")
    return d


def _run_stage(stage: str, args: argparse.Namespace) -> None:
    """Run one stage, translating any exception into an error event."""
    em = Emitter(stage)
    try:
        d = _song_dir(args)
        if stage == "render":
            from . import render
            render.run(d, args, em)
        elif stage == "separate":
            from . import separate
            separate.run(d, args, em)
        elif stage == "convert":
            from . import convert
            convert.run(d, args, em)
        elif stage == "mix":
            from . import mix
            mix.run(d, args, em)
        else:
            raise StageError(f"unknown stage {stage}")
    except StageError as e:
        em.error(e.message, e.detail)
        raise
    except KeyboardInterrupt:
        em.error("Cancelled")
        raise
    except Exception as e:  # noqa: BLE001 - the whole point is to report anything
        tb = traceback.format_exc()
        stderr_note(tb)
        # First line of the exception for the UI, the traceback tail as detail.
        msg = (str(e).strip().splitlines() or [type(e).__name__])[0]
        tail = "\n".join(tb.strip().splitlines()[-12:])
        em.error(f"{type(e).__name__}: {msg}" if msg else type(e).__name__, tail)
        raise StageError(msg, tail) from e


def cmd_run(args: argparse.Namespace) -> int:
    start = STAGE_ORDER.index(args.from_stage)
    for stage in STAGE_ORDER[start:]:
        if stage == "convert":
            if args.no_convert:
                Emitter("convert").log("Voice conversion skipped (--no-convert)")
                continue
            if not args.pth:
                # A missing model should not sink an overnight run; the mix
                # stage falls back to the model's own vocal.
                Emitter("convert").log("Voice conversion skipped (no --pth given)")
                continue
        _run_stage(stage, args)
    return 0


def main(argv: Optional[List[str]] = None) -> int:
    # Before anything heavy loads: stdout is ours, libraries get stderr.
    capture_library_output()
    paths.apply_cache_env()

    ap = build_parser()
    args = ap.parse_args(argv)

    if args.command == "check":
        from . import check
        return check.main()

    try:
        if args.command == "run":
            return cmd_run(args)
        _run_stage(args.command, args)
        return 0
    except StageError:
        return 1
    except KeyboardInterrupt:
        return 130


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
