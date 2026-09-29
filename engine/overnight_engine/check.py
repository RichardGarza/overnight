"""`check`: what is installed and which weights are on disk (SPEC 3.4).

Never downloads anything, exits 0 even when everything is missing. The JSON
goes to the protocol stream (stdout) as a single object; the Setup page shows
it as a checklist. Importing torch takes a second or two, the rest is file
system lookups.
"""

from __future__ import annotations

import importlib.util
import json
import platform
import sys
from typing import Any, Dict, List

from . import paths
from .progress import protocol_stream, stderr_note

# The files that prove a model is fully present. Partial downloads leave
# these missing, so "present" really means usable.
ACESTEP_FILES = [
    "ace_step_transformer/diffusion_pytorch_model.safetensors",
    "music_dcae_f8c8/diffusion_pytorch_model.safetensors",
    "music_vocoder/diffusion_pytorch_model.safetensors",
    "umt5-base/model.safetensors",
]
# demucs fetches htdemucs from the HuggingFace hub into $HF_HOME/hub; the
# weights are one ~80 MB blob (safetensors) under that repo folder.
DEMUCS_HF_DIR = "hub/models--adefossez--HTDemucs"
DEMUCS_MIN_BYTES = 50_000_000
RVC_ASSET_FILES = ["hubert_base.pt", "rmvpe.pt"]


def _installed(module: str) -> bool:
    try:
        return importlib.util.find_spec(module) is not None
    except (ImportError, ValueError):
        return False


def _torch_info() -> Dict[str, Any]:
    """torch version and the device that actually works. "mps" only if a tiny
    op runs on it, because is_available() can say yes on a machine where the
    Metal backend then fails."""
    info: Dict[str, Any] = {"torch": None, "device": "cpu", "mps": False}
    try:
        import torch
    except Exception as e:  # noqa: BLE001
        stderr_note(f"torch import failed: {e}")
        return info
    info["torch"] = torch.__version__
    try:
        if torch.backends.mps.is_available():
            x = torch.ones(2, device="mps") * 2
            if float(x.sum().item()) == 4.0:
                info["mps"] = True
                info["device"] = "mps"
    except Exception as e:  # noqa: BLE001
        stderr_note(f"mps probe failed: {e}")
    return info


def _demucs_present() -> bool:
    d = paths.hf_home() / DEMUCS_HF_DIR
    if not d.is_dir():
        return False
    for f in d.rglob("*"):
        try:
            if f.is_file() and f.stat().st_size >= DEMUCS_MIN_BYTES:
                return True
        except OSError:
            pass
    return False


def _models_present() -> Dict[str, bool]:
    ace = paths.acestep_dir()
    return {
        "acestep": all((ace / f).is_file() for f in ACESTEP_FILES),
        "demucs": _demucs_present(),
        "rvcAssets": all((paths.rvc_assets_dir() / f).is_file() for f in RVC_ASSET_FILES),
    }


def gather() -> Dict[str, Any]:
    t = _torch_info()
    rvc_py = paths.rvc_python_bin()
    models = _models_present()
    ffmpeg = paths.find_ffmpeg()
    acestep = _installed("acestep")
    demucs = _installed("demucs")

    problems: List[str] = []
    if t["torch"] is None:
        problems.append("torch is not installed in the engine venv; run Install engine.")
    elif not t["mps"]:
        problems.append("Metal (MPS) is not usable; renders will run on the CPU, which is very slow.")
    if not acestep:
        problems.append("ACE-Step is not installed; the render stage cannot run.")
    if not demucs:
        problems.append("demucs is not installed; the separate stage cannot run.")
    if rvc_py is None:
        problems.append("rvc-python is not installed; voice conversion is unavailable.")
    if not ffmpeg:
        problems.append("ffmpeg not found; install it with: brew install ffmpeg")
    if acestep and not models["acestep"]:
        problems.append("ACE-Step weights are not downloaded yet (about 8.3 GB); the first render fetches them.")
    if demucs and not models["demucs"]:
        problems.append("demucs weights are not downloaded yet (about 80 MB); the first separation fetches them.")
    if rvc_py is not None and not models["rvcAssets"]:
        problems.append("RVC base models (hubert, rmvpe; about 370 MB) download on first conversion.")

    out: Dict[str, Any] = {
        "python": platform.python_version(),
        "torch": t["torch"],
        "device": t["device"],
        "acestep": acestep,
        "demucs": demucs,
        "rvc": rvc_py is not None,
        "ffmpeg": ffmpeg,
        "models": models,
        "problems": problems,
        # Extras beyond the spec shape, for the Setup page and for debugging.
        # rvc-python lives in its own venv (setup.sh, SPEC 3.6).
        "rvcPython": str(rvc_py) if rvc_py else None,
        "venv": str(paths.venv_dir()),
        "modelsDir": str(paths.models_dir()),
        "executable": sys.executable,
    }
    return out


def main() -> int:
    out = gather()
    s = protocol_stream()
    s.write(json.dumps(out, ensure_ascii=False) + "\n")
    s.flush()
    return 0
