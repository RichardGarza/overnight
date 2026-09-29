"""Fetching model weights into the models folder, with progress events.

Nothing here runs unless a stage finds its weights missing (SPEC 3.6: setup
never downloads). The libraries' own downloaders print tqdm bars to stderr,
which the app only sees in engine.log, so we watch the destination grow and
emit `log` events the UI can show live.
"""

from __future__ import annotations

import os
import shutil
import threading
import time
import urllib.request
from pathlib import Path
from typing import Optional

from .progress import Emitter


def dir_bytes(path: Path) -> int:
    total = 0
    for root, _dirs, files in os.walk(path):
        for f in files:
            try:
                total += os.path.getsize(os.path.join(root, f))
            except OSError:
                pass
    return total


def _gb(n: float) -> str:
    return f"{n / 1e9:.2f} GB" if n >= 1e8 else f"{n / 1e6:.0f} MB"


def snapshot_with_progress(
    repo_id: str,
    local_dir: Path,
    em: Emitter,
    label: str,
    expected_bytes: Optional[int] = None,
    interval: float = 2.0,
) -> None:
    """huggingface_hub.snapshot_download into a flat folder (no cache
    symlinks), reporting the folder's size every couple of seconds. Resumable:
    the hub client skips complete files and continues partial ones."""
    from huggingface_hub import snapshot_download

    local_dir.mkdir(parents=True, exist_ok=True)
    if expected_bytes is None:
        expected_bytes = _repo_size(repo_id)

    result: dict = {}

    def work():
        try:
            snapshot_download(repo_id, local_dir=str(local_dir))
            result["ok"] = True
        except BaseException as e:  # noqa: BLE001 - re-raised on the main thread
            result["error"] = e

    t = threading.Thread(target=work, name="snapshot", daemon=True)
    t.start()
    last = -1
    while t.is_alive():
        t.join(interval)
        # Partial files live under local_dir/.cache/huggingface while in
        # flight, so counting the whole tree reflects real progress.
        have = dir_bytes(local_dir)
        if have != last:
            last = have
            if expected_bytes:
                em.log(f"{label}: {_gb(have)} of {_gb(expected_bytes)}")
            else:
                em.log(f"{label}: {_gb(have)}")
    if "error" in result:
        raise result["error"]
    em.log(f"{label}: download complete")


def _repo_size(repo_id: str) -> Optional[int]:
    """Total bytes of the files in a hub repo, or None if the API is not
    reachable (offline machines still get a working download attempt)."""
    try:
        from huggingface_hub import HfApi

        info = HfApi().model_info(repo_id, files_metadata=True)
        return sum((s.size or 0) for s in (info.siblings or []))
    except Exception:  # noqa: BLE001
        return None


def _ssl_context():
    """A uv-managed CPython does not always find the system's root
    certificates; certifi (pulled in by requests) always has a bundle."""
    import ssl

    try:
        import certifi

        return ssl.create_default_context(cafile=certifi.where())
    except ImportError:
        return ssl.create_default_context()


def fetch_url(url: str, dest: Path, em: Emitter, label: str, interval: float = 2.0) -> None:
    """Stream one file to `dest` via a temp file next to it, reporting bytes
    every couple of seconds. Plain urllib: nothing to configure, and the
    files we fetch this way are public."""
    dest.parent.mkdir(parents=True, exist_ok=True)
    tmp = dest.with_name(dest.name + ".part")
    req = urllib.request.Request(url, headers={"User-Agent": "overnight-engine"})
    with urllib.request.urlopen(req, timeout=60, context=_ssl_context()) as resp, open(tmp, "wb") as out:
        total = int(resp.headers.get("Content-Length") or 0)
        have = 0
        tick = time.monotonic()
        while True:
            chunk = resp.read(1 << 20)
            if not chunk:
                break
            out.write(chunk)
            have += len(chunk)
            if time.monotonic() - tick >= interval:
                tick = time.monotonic()
                em.log(f"{label}: {_gb(have)} of {_gb(total)}" if total else f"{label}: {_gb(have)}")
    if total and have != total:
        tmp.unlink(missing_ok=True)
        raise RuntimeError(f"{label}: short download ({have} of {total} bytes)")
    shutil.move(str(tmp), str(dest))
    em.log(f"{label}: download complete")
