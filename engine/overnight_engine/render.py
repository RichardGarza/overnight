"""`render`: song.json -> raw.wav through ACE-Step v1 (SPEC 3.1).

The model is ACE-Step/ACE-Step-v1-3.5B driven through `ACEStepPipeline`
from the ace_step package (pinned commit in setup.sh). One call:

    pipe(prompt=tags, lyrics=lyrics, audio_duration=sec, infer_step=steps,
         guidance_scale=g, manual_seeds=[seed], save_path="raw.wav", format="wav")

Precision: on Metal the pipeline refuses bf16 (upstream forces float32) and
this model is 3.5B parameters, which in float32 is ~14 GB of weights, more
than an 18 GB Mac can give the GPU. So on MPS we run float16, which fits in
about 7 GB, and fall back to float32 on the CPU if Metal fails outright.
OVERNIGHT_ACESTEP_DTYPE=float32|float16 overrides the choice.
"""

from __future__ import annotations

import argparse
import os
import random
import time
from pathlib import Path
from typing import Any, Dict, Optional

from . import paths
from .downloads import snapshot_with_progress
from .progress import Emitter, stderr_note
from .songs import load_song, wav_duration

REPO_ID = "ACE-Step/ACE-Step-v1-3.5B"
REPO_BYTES = 8_275_795_939  # what the hub reported when this was written
REQUIRED = [
    "ace_step_transformer/diffusion_pytorch_model.safetensors",
    "music_dcae_f8c8/diffusion_pytorch_model.safetensors",
    "music_vocoder/diffusion_pytorch_model.safetensors",
    "umt5-base/model.safetensors",
]


def weights_present(ckpt: Path) -> bool:
    return all((ckpt / f).is_file() for f in REQUIRED)


def ensure_weights(em: Emitter) -> Path:
    ckpt = paths.acestep_dir()
    if weights_present(ckpt):
        return ckpt
    em.log("ACE-Step weights missing; downloading ~8.3 GB (first run only)")
    snapshot_with_progress(REPO_ID, ckpt, em, "ACE-Step weights", expected_bytes=REPO_BYTES)
    if not weights_present(ckpt):
        raise RuntimeError(f"ACE-Step download finished but files are missing under {ckpt}")
    return ckpt


def _song_params(song: Dict[str, Any], args: argparse.Namespace) -> Dict[str, Any]:
    tags = (song.get("tags") or "").strip()
    lyrics = (song.get("lyrics") or "").strip()
    if not tags:
        raise ValueError("song.json has no tags (the style prompt)")
    if not lyrics:
        lyrics = "[inst]"
    duration = float(song.get("durationSec") or 150)
    steps = int(song.get("steps") or 60)
    seed = song.get("seed")
    if not isinstance(seed, int) or seed < 0:
        seed = random.randint(0, 2**31 - 1)
    return {
        "tags": tags,
        "lyrics": lyrics,
        "duration": max(5.0, min(600.0, duration)),
        "steps": max(1, min(200, steps)),
        "seed": int(seed),
        "guidance": float(getattr(args, "guidance", 15.0) or 15.0),
    }


def _pick_dtype(device: str) -> str:
    v = (os.environ.get("OVERNIGHT_ACESTEP_DTYPE") or "").strip().lower()
    if v in ("float32", "float16", "bfloat16"):
        return v
    return "float16" if device == "mps" else "float32"


def _make_progress_tqdm(em: Emitter, steps: int):
    """A stand-in for tqdm inside the pipeline module: the diffusion loop is
    `for i, t in tqdm(enumerate(timesteps), total=steps)`, so the iterable
    with `total == steps` is the one to report. Everything else passes through."""

    def fake_tqdm(iterable=None, total=None, **_kw):
        n = total
        if n is None and iterable is not None and hasattr(iterable, "__len__"):
            n = len(iterable)
        report = n == steps and n > 0

        def gen():
            for i, x in enumerate(iterable):
                yield x
                if report:
                    em.progress(5 + 85.0 * (i + 1) / n, f"Step {i + 1}/{n}")

        return gen()

    return fake_tqdm


def _save_wav_with_soundfile(self, target_wav, idx, save_path=None, sample_rate=48000, format="wav"):
    """Replacement for ACEStepPipeline.save_wav_file. torchaudio 2.9+ has no
    file backends of its own any more (it wants torchcodec, which needs its
    own ffmpeg build), so write the float tensor with soundfile instead.
    32-bit float keeps the model's output as is; nothing clips before mix."""
    import soundfile as sf

    path = str(save_path) if save_path else f"output_{idx}.wav"
    wav = target_wav.detach().float().cpu().numpy()  # [channels, samples]
    sf.write(path, wav.T, int(sample_rate), subtype="FLOAT")
    return path


def _render_once(
    ckpt: Path,
    device: str,
    dtype: str,
    p: Dict[str, Any],
    out: Path,
    em: Emitter,
) -> None:
    import torch
    from acestep import pipeline_ace_step as pas

    # The pipeline reads ACE_PIPELINE_DTYPE in its constructor and it is the
    # only way to get anything but float32 on MPS.
    os.environ["ACE_PIPELINE_DTYPE"] = dtype
    pas.tqdm = _make_progress_tqdm(em, p["steps"])
    pas.ACEStepPipeline.save_wav_file = _save_wav_with_soundfile

    pipe = pas.ACEStepPipeline(checkpoint_dir=str(ckpt), dtype=dtype)
    # The constructor picks cuda > mps > cpu on its own; we want the app's choice.
    pipe.device = torch.device(device)
    if device == "cpu":
        pipe.dtype = torch.float32
    em.progress(2, f"Loading weights ({device}, {dtype})")
    t0 = time.monotonic()
    pipe.load_checkpoint(str(ckpt))
    em.log(f"Weights loaded in {time.monotonic() - t0:.1f} s")
    em.progress(5, "Generating")

    tmp = out.with_name("raw.partial.wav")
    tmp.unlink(missing_ok=True)
    result = pipe(
        format="wav",
        audio_duration=p["duration"],
        prompt=p["tags"],
        lyrics=p["lyrics"],
        infer_step=p["steps"],
        guidance_scale=p["guidance"],
        manual_seeds=[p["seed"]],
        save_path=str(tmp),
    )
    stderr_note(f"ace-step result: {result[-1] if result else result}")
    # The pipeline drops a sidecar json next to the wav; the song folder has
    # its own record, so remove it.
    tmp.with_name("raw.partial_input_params.json").unlink(missing_ok=True)
    if not tmp.is_file():
        raise RuntimeError("ACE-Step finished without writing a wav")
    em.progress(97, "Writing raw.wav")
    os.replace(tmp, out)


def _is_mps_failure(e: BaseException) -> bool:
    s = str(e).lower()
    return "mps" in s or "metal" in s or "out of memory" in s


def run(song_dir: Path, args: argparse.Namespace, em: Emitter) -> None:
    out = song_dir / "raw.wav"
    if out.is_file() and not getattr(args, "force", False):
        em.start("raw.wav already rendered")
        dur = wav_duration(out)
        em.done(f"raw.wav {dur:.1f} s (kept)" if dur else "raw.wav kept", detail="0.0 s")
        return

    song = load_song(song_dir)
    p = _song_params(song, args)
    em.start("Loading ACE-Step")
    em.log(f"{p['duration']:.0f} s, {p['steps']} steps, guidance {p['guidance']:g}, seed {p['seed']}")
    ckpt = ensure_weights(em)

    device = paths.device_name()
    dtype = _pick_dtype(device)
    em.log(f"Device: {device} ({dtype})")
    t0 = time.monotonic()
    try:
        _render_once(ckpt, device, dtype, p, out, em)
    except Exception as e:  # noqa: BLE001
        if device != "mps" or not _is_mps_failure(e):
            raise
        # Metal gave up (usually memory). Say so and finish on the CPU rather
        # than lose the night's run. Slow, but it completes.
        stderr_note(f"MPS render failed: {e!r}")
        em.log(f"MPS failed ({type(e).__name__}); retrying on CPU, this is slow")
        _free_torch_memory()
        _render_once(ckpt, "cpu", "float32", p, out, em)

    elapsed = time.monotonic() - t0
    dur = wav_duration(out) or 0.0
    em.done(f"raw.wav {dur:.1f} s", detail=f"{elapsed:.1f} s")


def _free_torch_memory() -> None:
    import gc

    gc.collect()
    try:
        import torch

        if torch.backends.mps.is_available():
            torch.mps.empty_cache()
    except Exception:  # noqa: BLE001
        pass
