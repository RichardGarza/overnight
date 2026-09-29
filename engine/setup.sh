#!/bin/bash
# Overnight engine setup: makes the Python environment the audio pipeline runs in.
#
#   setup.sh <venv-dir> <models-dir>
#
# Idempotent: run it again and it only does what is missing. It never downloads
# model weights (those arrive on first use, with progress events); it only
# installs Python packages. Every step prints one JSON line in the engine's
# progress format so the app can show it live:
#   {"stage":"setup","event":"log","message":"..."}
#
# Why uv and not pip: it resolves and installs the 3 GB torch stack in a couple
# of minutes instead of ten, and it can fetch a Python 3.10 on its own, which
# matters because fairseq (pulled in by rvc-python) does not build on 3.11+.

set -u

VENV="${1:-}"
MODELS="${2:-}"
if [ -z "$VENV" ] || [ -z "$MODELS" ]; then
  echo '{"stage":"setup","event":"error","message":"usage: setup.sh <venv-dir> <models-dir>"}'
  exit 2
fi

ENGINE_DIR="$(cd "$(dirname "$0")" && pwd)"
PY_VERSION="3.10"
# The ACE-Step commit this engine was verified against (main, 2026-02-15).
# Later commits may change the pipeline API; bump deliberately.
ACESTEP_COMMIT="1bee4c9f5b43e30995f8d4d33b3919197ce1bd68"
ACESTEP_SPEC="ace_step @ git+https://github.com/ace-step/ACE-Step.git@${ACESTEP_COMMIT}"
RVC_VENV="${VENV}-rvc"
# fairseq tag v0.12.2, pinned to its commit so a moved tag cannot change the build.
FAIRSEQ_SPEC="fairseq @ git+https://github.com/facebookresearch/fairseq.git@4a388e64cd646ed7d7ad8de8fae55df2b8eea91d"

# ------------------------------------------------------------------ helpers

# Print a protocol line. Only quotes and backslashes need escaping for the
# short messages we emit; anything fancier goes through the Python emitter.
emit() {
  local event="$1" msg="$2"
  msg="${msg//\\/\\\\}"
  msg="${msg//\"/\\\"}"
  local at
  at="$(date +%s)000"
  printf '{"stage":"setup","event":"%s","message":"%s","at":%s}\n' "$event" "$msg" "$at"
}

fail() {
  emit error "$1"
  exit 1
}

# Where uv might be when we are launched from a Finder app with no PATH.
find_uv() {
  if [ -n "${UV_BIN:-}" ] && [ -x "$UV_BIN" ]; then echo "$UV_BIN"; return; fi
  local c
  for c in uv "$HOME/.local/bin/uv" "$HOME/.cargo/bin/uv" /opt/homebrew/bin/uv /usr/local/bin/uv; do
    if command -v "$c" >/dev/null 2>&1; then command -v "$c"; return; fi
  done
  return 1
}

# Run an install step, streaming uv's own output to stderr (the app appends
# stderr to engine.log) and reporting the outcome as a protocol line.
step() {
  local label="$1"; shift
  emit log "$label"
  if "$@" 1>&2; then
    return 0
  else
    fail "$label failed (see engine.log)"
  fi
}

# ------------------------------------------------------------------ main

emit start "Setting up the Overnight engine"

UV="$(find_uv)" || fail "uv not found. Install it with: curl -LsSf https://astral.sh/uv/install.sh | sh"
emit log "Using uv at $UV"

mkdir -p "$MODELS" || fail "Cannot create models folder $MODELS"
mkdir -p "$(dirname "$VENV")" || fail "Cannot create $(dirname "$VENV")"

# Keep uv's own caches out of ~/.cache so a wipe of the data folder is a
# clean slate. Python downloads live next to the venv for the same reason.
export UV_CACHE_DIR="$MODELS/.uv-cache"
export UV_PYTHON_INSTALL_DIR="$MODELS/.uv-python"

# 1. The venv. `uv venv` on an existing dir would wipe it, so only create once.
if [ -x "$VENV/bin/python" ]; then
  emit log "venv already exists at $VENV"
else
  step "Creating Python $PY_VERSION venv at $VENV" "$UV" venv --python "$PY_VERSION" "$VENV"
fi
PY="$VENV/bin/python"

# 2. torch first, on its own, so the resolver pins the rest of the stack to
# the torch we got (PyPI's arm64 wheels run on Metal via MPS out of the box).
step "Installing torch and torchaudio" "$UV" pip install --python "$PY" torch torchaudio

# 3. The music model, the stem separator and the small stuff.
step "Installing ACE-Step (${ACESTEP_COMMIT:0:7}), demucs, Pillow, soundfile, numpy" \
  "$UV" pip install --python "$PY" "$ACESTEP_SPEC" demucs Pillow soundfile numpy

# 4. The engine itself, as a normal (non-editable) install so the venv carries
# its own copy and keeps working when the app bundle moves. Re-installed every
# run so an updated engine source lands.
step "Installing the Overnight engine package" \
  "$UV" pip install --python "$PY" --reinstall-package overnight-engine "$ENGINE_DIR"

# 5. rvc-python. It pins numpy<=1.23.5 and fairseq 0.12.2, which the ACE-Step
# stack cannot live with, so it gets a venv of its own (SPEC 3.6). The engine's
# convert stage runs `<venv>-rvc/bin/python` and `check` reports both.
if [ -x "$RVC_VENV/bin/python" ]; then
  emit log "rvc venv already exists at $RVC_VENV"
else
  step "Creating Python $PY_VERSION venv for rvc-python at $RVC_VENV" "$UV" venv --python "$PY_VERSION" "$RVC_VENV"
fi
RVC_PY="$RVC_VENV/bin/python"
step "Installing torch and torchaudio for rvc" "$UV" pip install --python "$RVC_PY" torch torchaudio
# pyworld and fairseq ship no arm64 wheels and build from source with setup.py
# files that import numpy and Cython without declaring them, so those must be
# in the venv before the build runs (hence --no-build-isolation). rvc-python
# pins numpy<=1.23.5, so that is the numpy we put there. pyworld 0.3.5 needs
# Cython 3 syntax. The fairseq 0.12.2 sdist on PyPI is missing its C++
# sources (clib/libbase), so it comes from the matching git tag instead.
step "Installing rvc-python build helpers" \
  "$UV" pip install --python "$RVC_PY" "pip" "setuptools<70" "wheel" "cython>=3" "numpy<=1.23.5"
step "Building pyworld" "$UV" pip install --python "$RVC_PY" --no-build-isolation pyworld
step "Building fairseq 0.12.2 from source (about a minute)" \
  "$UV" pip install --python "$RVC_PY" --no-build-isolation "$FAIRSEQ_SPEC"
step "Installing rvc-python" "$UV" pip install --python "$RVC_PY" --no-build-isolation rvc-python
# torch's install swaps in a new setuptools; pyworld imports pkg_resources
# from the old one at runtime, so put it back last.
step "Pinning setuptools for pyworld" "$UV" pip install --python "$RVC_PY" "setuptools<70"
step "Installing the Overnight engine package into the rvc venv" \
  "$UV" pip install --python "$RVC_PY" --no-deps --reinstall-package overnight-engine "$ENGINE_DIR"
# setuptools leaves its build tree and egg-info next to the source; in the
# repo those would show up as untracked files, in the app bundle as junk.
rm -rf "$ENGINE_DIR/build" "$ENGINE_DIR"/*.egg-info

# 6. Report what we have. `check` never downloads and exits 0 even when
# things are missing; its JSON goes to stdout for the app to read.
emit log "Checking the engine"
OVERNIGHT_MODELS="$MODELS" OVERNIGHT_DATA="$(dirname "$VENV")" "$PY" -m overnight_engine check \
  || fail "engine check failed"

emit done "Engine setup complete"
