"""Where things are: the data folder, the models folder, the venvs, ffmpeg.

Nothing the engine downloads may land in ~/.cache. Every library we use has
an environment variable for its cache, and `apply_cache_env` sets them all to
subfolders of OVERNIGHT_MODELS before the libraries are imported (they read
the variables at import time, so this has to happen first).
"""

from __future__ import annotations

import os
import shutil
import sys
from pathlib import Path
from typing import Optional

APP_DIR_NAME = "com.overnight.studio"


def data_dir() -> Path:
    v = os.environ.get("OVERNIGHT_DATA")
    if v:
        return Path(v).expanduser()
    return Path.home() / "Library" / "Application Support" / APP_DIR_NAME


def models_dir() -> Path:
    v = os.environ.get("OVERNIGHT_MODELS")
    if v:
        return Path(v).expanduser()
    return data_dir() / "models"


def device_name() -> str:
    """"mps" or "cpu". Rust resolves "auto"; we still tolerate it."""
    v = (os.environ.get("OVERNIGHT_DEVICE") or "auto").strip().lower()
    if v in ("mps", "cpu"):
        return v
    try:
        import torch  # noqa: WPS433 - only when asked to auto-detect

        return "mps" if torch.backends.mps.is_available() else "cpu"
    except Exception:  # noqa: BLE001
        return "cpu"


# Layout under the models folder. Kept flat and named so a person can see
# what is taking the space.
def acestep_dir() -> Path:
    return models_dir() / "acestep"


def torch_home() -> Path:
    # demucs fetches its weights through torch.hub -> $TORCH_HOME/hub/checkpoints
    return models_dir() / "torch"


def hf_home() -> Path:
    return models_dir() / "hf"


def rvc_assets_dir() -> Path:
    # hubert_base.pt and rmvpe.pt, the shared RVC base models
    return models_dir() / "rvc"


def cache_dir() -> Path:
    return models_dir() / "cache"


def apply_cache_env() -> None:
    """Point every cache we know about into the models folder."""
    m = models_dir()
    env = {
        "HF_HOME": str(hf_home()),
        "HF_HUB_CACHE": str(hf_home() / "hub"),
        "TORCH_HOME": str(torch_home()),
        "ACE_STEP_CHECKPOINTS": str(acestep_dir()),
        "XDG_CACHE_HOME": str(cache_dir()),
        "NUMBA_CACHE_DIR": str(cache_dir() / "numba"),
        "MPLCONFIGDIR": str(cache_dir() / "matplotlib"),
        "HF_HUB_DISABLE_TELEMETRY": "1",
        "TOKENIZERS_PARALLELISM": "false",
        # MPS lacks a few ops ACE-Step and demucs touch; fall back per op.
        "PYTORCH_ENABLE_MPS_FALLBACK": "1",
    }
    for k, v in env.items():
        os.environ.setdefault(k, v)
    for d in (m, hf_home(), torch_home(), cache_dir(), cache_dir() / "numba", cache_dir() / "matplotlib"):
        try:
            d.mkdir(parents=True, exist_ok=True)
        except OSError:
            pass
    # demucs shells out to a bare `ffmpeg` when its native reader cannot
    # open a file; an app launched from Finder has no Homebrew on PATH.
    ff = find_ffmpeg()
    if ff:
        ffdir = os.path.dirname(ff)
        if ffdir not in os.environ.get("PATH", "").split(os.pathsep):
            os.environ["PATH"] = ffdir + os.pathsep + os.environ.get("PATH", "")


# --------------------------------------------------------------- binaries

WELL_KNOWN_BIN_DIRS = ["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin", "/bin"]


def find_ffmpeg() -> Optional[str]:
    """OVERNIGHT_FFMPEG, then PATH, then the usual Homebrew places (an app
    launched from Finder has almost no PATH)."""
    v = os.environ.get("OVERNIGHT_FFMPEG")
    if v and os.access(v, os.X_OK):
        return v
    found = shutil.which("ffmpeg")
    if found:
        return found
    for d in WELL_KNOWN_BIN_DIRS:
        p = os.path.join(d, "ffmpeg")
        if os.access(p, os.X_OK):
            return p
    return None


def venv_dir() -> Path:
    """The venv this interpreter runs in."""
    return Path(sys.prefix)


def rvc_python_bin() -> Optional[Path]:
    """The interpreter that has rvc-python: this one, or the `<venv>-rvc`
    sibling that setup.sh creates (SPEC 3.6). OVERNIGHT_RVC_PYTHON overrides."""
    v = os.environ.get("OVERNIGHT_RVC_PYTHON")
    if v and os.access(v, os.X_OK):
        return Path(v)
    here = venv_dir()
    candidates = []
    if here.name.endswith("-rvc"):
        candidates.append(here)
    else:
        candidates.append(here.with_name(here.name + "-rvc"))
    candidates.append(here)
    for c in candidates:
        py = c / "bin" / "python"
        if py.exists() and has_package(c, "rvc_python"):
            return py
    return None


def has_package(venv: Path, package: str) -> bool:
    """Is `package` installed in `venv`? Checked on disk, without importing,
    because importing rvc_python drags in torch and fairseq (seconds)."""
    lib = venv / "lib"
    if not lib.is_dir():
        return False
    for pydir in lib.glob("python3*"):
        sp = pydir / "site-packages"
        if (sp / package).is_dir() or (sp / f"{package}.py").exists():
            return True
    return False
