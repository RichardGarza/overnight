"""`separate`: raw.wav -> vocals.wav + instrumental.wav with demucs (SPEC 3.1).

htdemucs gives four stems; we keep the vocal and sum the other three into
the instrumental, which is what demucs' own `--two-stems=vocals` does. The
model weights come from the HuggingFace hub through demucs' loader, so they
land in HF_HOME (inside the models folder, see paths.apply_cache_env).

On Metal demucs already runs its STFT on the CPU (complex numbers are not
supported there) and the transformer on the GPU. If Metal fails anyway we
redo the split on the CPU; a few minutes rather than a lost night.
"""

from __future__ import annotations

import argparse
import time
from pathlib import Path

from . import paths
from .progress import Emitter, stderr_note
from .songs import wav_duration

MODEL = "htdemucs"


def _split(raw: Path, device: str, em: Emitter):
    from demucs.api import Separator

    def on_chunk(info: dict) -> None:
        # Called at the start and end of each segment of each shift/model.
        if info.get("state") != "end":
            return
        length = float(info.get("audio_length") or 0)
        if length <= 0:
            return
        offset = float(info.get("segment_offset") or 0)
        models = max(1, int(info.get("models") or 1))
        model_idx = int(info.get("model_idx_in_bag") or 0)
        frac = (model_idx + min(1.0, offset / length)) / models
        em.progress(10 + 80 * frac, "Separating")

    sep = Separator(model=MODEL, device=device, shifts=1, split=True, overlap=0.25,
                    progress=False, callback=on_chunk)
    em.progress(10, "Separating")
    _origin, stems = sep.separate_audio_file(raw)
    return sep.samplerate, stems


def _write_wav(tensor, path: Path, sr: int) -> None:
    """[channels, samples] float tensor -> 32-bit float wav via soundfile.
    demucs' own save_audio goes through torchaudio, which in 2.9+ has no
    backend without torchcodec; soundfile is already a dependency."""
    import soundfile as sf

    sf.write(str(path), tensor.detach().float().cpu().numpy().T, int(sr), subtype="FLOAT")


def run(song_dir: Path, args: argparse.Namespace, em: Emitter) -> None:
    raw = song_dir / "raw.wav"
    if not raw.is_file():
        raise FileNotFoundError("No raw.wav to separate; render first")
    em.start(f"Loading demucs ({MODEL})")
    device = paths.device_name()
    em.log(f"Device: {device}")
    t0 = time.monotonic()

    try:
        sr, stems = _split(raw, device, em)
    except Exception as e:  # noqa: BLE001
        if device != "mps":
            raise
        stderr_note(f"MPS separation failed: {e!r}")
        em.log(f"MPS failed ({type(e).__name__}); retrying on CPU")
        sr, stems = _split(raw, "cpu", em)

    if "vocals" not in stems:
        raise RuntimeError(f"demucs returned no vocals stem (got {sorted(stems)})")
    vocals = stems["vocals"]
    others = [v for k, v in stems.items() if k != "vocals"]
    instrumental = sum(others[1:], others[0]) if others else vocals * 0

    em.progress(92, "Writing stems")
    vpath = song_dir / "vocals.wav"
    ipath = song_dir / "instrumental.wav"
    # Write via temp names so a crash mid-write never leaves a truncated stem
    # that a later `run --from mix` would trust.
    vtmp = song_dir / "vocals.partial.wav"
    itmp = song_dir / "instrumental.partial.wav"
    _write_wav(vocals, vtmp, sr)
    _write_wav(instrumental, itmp, sr)
    vtmp.replace(vpath)
    itmp.replace(ipath)

    dur = wav_duration(vpath) or 0.0
    em.done(f"vocals.wav + instrumental.wav {dur:.1f} s", detail=f"{time.monotonic() - t0:.1f} s")
