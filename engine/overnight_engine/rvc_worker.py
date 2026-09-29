"""The RVC half of `convert`, run inside the rvc venv by convert.py.

    python -m overnight_engine.rvc_worker --in vocals.wav --out out.wav --pth model.pth [...]

Emits only `progress` and `log` lines, plus a single `error` line on
failure (the parent stage owns start/done). rvc-python fetches its two base
models (hubert_base.pt, rmvpe.pt) into its own package folder with no
progress; we fetch them ourselves into <models>/rvc with progress events and
point the package at that folder with a symlink, so the models folder stays
the one place weights live.
"""

from __future__ import annotations

import argparse
import pickle
import sys
import traceback
from pathlib import Path

from . import paths
from .downloads import fetch_url
from .progress import Emitter, capture_library_output, stderr_note

BASE_MODELS = {
    "hubert_base.pt": "https://huggingface.co/Daswer123/RVC_Base/resolve/main/hubert_base.pt",
    "rmvpe.pt": "https://huggingface.co/Daswer123/RVC_Base/resolve/main/rmvpe.pt",
}


def ensure_base_models(em: Emitter) -> Path:
    d = paths.rvc_assets_dir()
    d.mkdir(parents=True, exist_ok=True)
    for name, url in BASE_MODELS.items():
        if not (d / name).is_file():
            em.log(f"Downloading RVC base model {name}")
            fetch_url(url, d / name, em, name)
    return d


def link_package_assets(assets: Path) -> None:
    """Make rvc_python/base_model a symlink to our assets folder. If a real
    folder is already there (an earlier download), move its files across."""
    import rvc_python

    pkg = Path(rvc_python.__file__).parent
    target = pkg / "base_model"
    if target.is_symlink():
        if target.resolve() == assets.resolve():
            return
        target.unlink()
    elif target.is_dir():
        for f in target.iterdir():
            if f.is_file() and not (assets / f.name).exists():
                f.replace(assets / f.name)
            else:
                f.unlink()
        target.rmdir()
    target.symlink_to(assets, target_is_directory=True)


def _allow_full_unpickle(em: Emitter) -> None:
    """RVC checkpoints predate torch's weights_only default (2.6+) and some
    carry plain Python objects the safe unpickler refuses. The user chose
    this file on purpose (SPEC 8: the app runs whatever .pth they point at),
    so when the safe load refuses, retry the way the RVC tools themselves
    load it, and say so in the log."""
    import torch

    original = torch.load

    def load(*a, **kw):
        # fairseq passes an open file, so remember where it was: the failed
        # first attempt leaves the position mid-stream.
        src = a[0] if a else kw.get("f")
        pos = src.tell() if hasattr(src, "tell") else None
        try:
            return original(*a, **kw)
        except pickle.UnpicklingError:
            if kw.get("weights_only") is False:
                raise
            stderr_note("safe torch.load refused the checkpoint; retrying with weights_only=False")
            if pos is not None:
                src.seek(pos)
            kw["weights_only"] = False
            return original(*a, **kw)

    torch.load = load


def build_parser() -> argparse.ArgumentParser:
    ap = argparse.ArgumentParser(prog="overnight_engine.rvc_worker")
    ap.add_argument("--in", dest="src", required=True)
    ap.add_argument("--out", dest="dst", required=True)
    ap.add_argument("--pth", required=True)
    ap.add_argument("--index", default="")
    ap.add_argument("--pitch", type=int, default=0)
    ap.add_argument("--method", default="rmvpe")
    ap.add_argument("--index-rate", type=float, default=0.66)
    ap.add_argument("--protect", type=float, default=0.33)
    ap.add_argument("--filter-radius", type=int, default=3)
    ap.add_argument("--rms-mix-rate", type=float, default=0.25)
    return ap


def convert(args: argparse.Namespace, em: Emitter) -> None:
    assets = ensure_base_models(em)
    em.progress(5, "Loading rvc-python")
    device = paths.device_name()

    import rvc_python.infer as infer_mod
    from rvc_python.configs.config import Config

    link_package_assets(assets)
    # hubert_base.pt is a fairseq checkpoint that torch's safe loader refuses
    # (it pickles a fairseq Dictionary), so this is needed for every run.
    _allow_full_unpickle(em)
    # We already fetched the base models; the package's own downloader would
    # also pull a 360 MB onnx copy of rmvpe that the torch path never uses.
    infer_mod.download_rvc_models = lambda _dir: None
    if device == "cpu":
        # Config picks MPS whenever it exists; honour the app's choice.
        Config.has_mps = staticmethod(lambda: False)

    em.log(f"Device: {device}")
    rvc = infer_mod.RVCInference(models_dir=str(assets), device=device if device == "cpu" else "mps:0")
    em.progress(15, f"Loading {Path(args.pth).name}")
    try:
        rvc.load_model(args.pth, version="v2", index_path=args.index or "")
    except Exception as e:  # noqa: BLE001 - torch and pickle raise all sorts on a bad file
        stderr_note(f"load_model failed: {type(e).__name__}: {e}")
        raise RuntimeError(f"{Path(args.pth).name} is not a usable RVC model file ({type(e).__name__})") from None
    rvc.set_params(
        f0method=args.method,
        f0up_key=args.pitch,
        index_rate=args.index_rate,
        filter_radius=args.filter_radius,
        rms_mix_rate=args.rms_mix_rate,
        protect=args.protect,
    )
    em.progress(30, f"Converting ({args.method}, {args.pitch:+d} st)")
    rvc.infer_file(args.src, args.dst)
    if not Path(args.dst).is_file():
        raise RuntimeError("rvc-python returned without writing the output file")
    em.progress(98, "Converted")


def main(argv=None) -> int:
    capture_library_output()
    paths.apply_cache_env()
    args = build_parser().parse_args(argv)
    em = Emitter("convert")
    try:
        convert(args, em)
        return 0
    except Exception as e:  # noqa: BLE001 - one error line, whatever it was
        tb = traceback.format_exc()
        stderr_note(tb)
        msg = (str(e).strip().splitlines() or [type(e).__name__])[0]
        # Our own RuntimeErrors are already written for a person; library
        # exceptions get their type in front so the log line means something.
        if not isinstance(e, RuntimeError) or not msg:
            msg = f"{type(e).__name__}: {msg}" if msg else type(e).__name__
        em.error(msg, "\n".join(tb.strip().splitlines()[-12:]))
        return 1


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
