# Overnight engine

The Python half of Overnight: ACE-Step renders the song, demucs splits the
vocal off, rvc-python (optional) re-sings it, ffmpeg mixes and masters, Pillow
draws the cover. The Rust backend drives it as
`<venv>/bin/python -m overnight_engine <command> --dir <songdir>` and reads one
JSON object per line from stdout (SPEC 3.3). This file is the engine-only
notes: how to run each command by hand, what is installed where, and what it
cost on the development Mac.

## Layout on disk

Everything lives under `~/Library/Application Support/com.overnight.studio/`:

```
venv/            Python 3.10 (uv-managed), torch 2.14 + ACE-Step + demucs + the engine
venv-rvc/        Python 3.10, torch 2.14 + rvc-python 0.1.5 + fairseq 0.12.2 + the engine
models/
  acestep/       ACE-Step/ACE-Step-v1-3.5B, flat (8.3 GB), first render fetches it
  hf/            HF_HOME: demucs' htdemucs comes through the hub (89 MB)
  rvc/           hubert_base.pt + rmvpe.pt (370 MB), first convert fetches them
  torch/         TORCH_HOME (unused today, reserved for torch.hub downloads)
  cache/         XDG_CACHE_HOME, numba and matplotlib caches
  .uv-cache/     uv's wheel cache, .uv-python/ its Python 3.10 build
songs/<id>/      song.json, raw.wav, vocals.wav, instrumental.wav, final.wav, final.mp3, cover.png
voices/          where the user drops RVC .pth / .index files
```

Nothing goes to `~/.cache`: `paths.apply_cache_env()` sets `HF_HOME`,
`HF_HUB_CACHE`, `TORCH_HOME`, `ACE_STEP_CHECKPOINTS`, `XDG_CACHE_HOME`,
`NUMBA_CACHE_DIR` and `MPLCONFIGDIR` into `models/` before any library is
imported, and `setup.sh` sets `UV_CACHE_DIR` / `UV_PYTHON_INSTALL_DIR` there
too. rvc-python wants its base models inside its own package folder; the
engine fetches them into `models/rvc` and symlinks `rvc_python/base_model`
to it.

Why two venvs: rvc-python pins `numpy<=1.23.5`, `fairseq==0.12.2` and
`faiss-cpu==1.7.3`; ACE-Step's stack (transformers 4.50, librosa 0.11,
matplotlib 3.10) does not live with numpy 1.23, and fairseq's build is
fragile enough that keeping it out of the render venv is worth the second
600 MB of torch. `convert` spawns `venv-rvc/bin/python -m
overnight_engine.rvc_worker` and relays its progress lines. `check` reports
the worker interpreter as `rvcPython`.

## setup.sh

```
engine/setup.sh "$HOME/Library/Application Support/com.overnight.studio/venv" \
                "$HOME/Library/Application Support/com.overnight.studio/models"
```

Idempotent; prints `{"stage":"setup",...}` lines; installs, never downloads
weights. Pins:

- torch / torchaudio: latest PyPI arm64 wheels (2.14.0 / 2.11.0 at the time of writing)
- ACE-Step: `git+https://github.com/ace-step/ACE-Step.git@1bee4c9f5b43e30995f8d4d33b3919197ce1bd68`
  (main as of 2026-02-15, package `ace_step` 0.2.0, model `ACE-Step/ACE-Step-v1-3.5B`)
- demucs 4.0.1, Pillow, soundfile, numpy
- rvc-python 0.1.5 with fairseq built from
  `git+https://github.com/facebookresearch/fairseq.git@4a388e64cd646ed7d7ad8de8fae55df2b8eea91d`
  (tag v0.12.2; the PyPI sdist of 0.12.2 is missing its C++ sources), pyworld
  built with Cython 3, `setuptools<70` kept for pyworld's `pkg_resources` import

Measured on the M3 Pro (fast connection, warm uv cache in the second run):
first run 25 s for the render venv (torch + ACE-Step stack) and about 2 min for
the rvc venv, of which fairseq's build is 36 s and pyworld's 8 s. Re-runs take
under 10 s.

ACE-Step 1.5 (January 2026) was looked at and not used: it wants Python
3.11-3.12, a different pipeline API and an extra LM, and this engine was not
going to be verified on MPS with it in the time available. The v1 API below is
what the spec's `steps` / `guidance` knobs map to.

## Commands, by hand

Set the environment the app would set (`OVERNIGHT_DEVICE` is `mps` or `cpu`;
`PYTORCH_ENABLE_MPS_FALLBACK=1` and `PYTHONUNBUFFERED=1` are what Rust passes):

```
export OVERNIGHT_DATA="$HOME/Library/Application Support/com.overnight.studio"
export OVERNIGHT_MODELS="$OVERNIGHT_DATA/models"
export OVERNIGHT_DEVICE=mps PYTORCH_ENABLE_MPS_FALLBACK=1 PYTHONUNBUFFERED=1
PY="$OVERNIGHT_DATA/venv/bin/python"
SONG="$OVERNIGHT_DATA/songs/20260929-111500-t3st"   # a folder with song.json

$PY -m overnight_engine check                              # JSON, exit 0 always
$PY -m overnight_engine render   --dir "$SONG" --guidance 15 [--force]
$PY -m overnight_engine separate --dir "$SONG"
$PY -m overnight_engine convert  --dir "$SONG" --pth ~/…/voice.pth [--index …] [--pitch 0] [--method rmvpe]
$PY -m overnight_engine mix      --dir "$SONG" [--vocals-gain-db 0] [--bitrate 320]
$PY -m overnight_engine run      --dir "$SONG" [--from render|separate|convert|mix] [--no-convert] [+ flags]
```

Stdout is the protocol only (`progress.capture_library_output` moves fd 1 to a
private handle and points fd 1 at stderr before torch loads); stderr is the
log. `render` skips when `raw.wav` exists unless `--force`. `run` skips the
convert stage with a `log` line when `--no-convert` is given or no `--pth` is
given (a missing voice model should not sink a night's run). Every stage
writes to `*.partial.*` and renames at the end, so a killed run never leaves a
half file that a resume would trust.

The ACE-Step call (`render.py`):

```python
from acestep.pipeline_ace_step import ACEStepPipeline
pipe = ACEStepPipeline(checkpoint_dir="<models>/acestep", dtype="float16")  # ACE_PIPELINE_DTYPE
pipe.device = torch.device("mps")
pipe.load_checkpoint("<models>/acestep")
pipe(format="wav", audio_duration=30, prompt=tags, lyrics=lyrics, infer_step=60,
     guidance_scale=15.0, manual_seeds=[seed], save_path="raw.partial.wav")
```

Two things are patched in the pipeline module before the call: `tqdm` is
replaced so each diffusion step becomes a `progress` event, and
`ACEStepPipeline.save_wav_file` is replaced with a soundfile writer, because
torchaudio 2.9+ has no file backend without torchcodec (which needs its own
ffmpeg build). Precision: upstream forces float32 on MPS, but the 3.5B model
in float32 is 14 GB of weights and an 18 GB Mac cannot give that to Metal, so
the engine runs float16 there (7 GB) and falls back to float32 on the CPU if
Metal throws. `OVERNIGHT_ACESTEP_DTYPE=float32|float16` overrides.

demucs (`separate.py`): `demucs.api.Separator(model="htdemucs", device=...)`,
`separate_audio_file(raw.wav)`, instrumental = drums + bass + other. Stems are
written as 32-bit float wav with soundfile (same torchaudio reason).

rvc-python (`rvc_worker.py`, in venv-rvc): `RVCInference(models_dir, device)`,
`load_model(pth, index_path)`, `set_params(f0method, f0up_key, index_rate,
filter_radius, rms_mix_rate, protect)`, `infer_file(in, out)`. `torch.load` is
wrapped to retry with `weights_only=False` when the safe loader refuses:
hubert_base.pt is a fairseq checkpoint and torch 2.6+ refuses it otherwise.

ffmpeg (`mix.py`): stems mixed with `amix` after `aresample=48000` and a
`volume=<gain>dB` on the vocal; loudnorm in two passes (measure, then apply
`measured_*` with `linear=true`) to I=-14 LUFS, TP=-1 dBTP, LRA=11; mp3 320k
via libmp3lame with ID3v2.3 title / artist (persona name) / album
("Overnight") / date / genre / comment (the theme) and cover.png embedded as
a JPEG front picture. Cover (`cover.py`): Pillow + numpy, Barlow Condensed
Bold (OFL, `fonts/OFL.txt`), seeded from the song id.

## What was measured (MacBook Pro M3 Pro, 18 GB, macOS 26)

Test song: 30 s, two sections (verse + chorus), the persona tags plus three
descriptors, seed 1234567, 60 steps, guidance 15, MPS float16.

| Stage | Wall clock | Notes |
|---|---|---|
| render, 60 steps, 30 s | 97 s (process start to `done`) | imports 4.6 s warm / 47 s cold, weights 25 s, 60 diffusion steps 46 s (0.5 s/step outside the CFG window, 1.05 s/step inside it, steps 15-45), decode + write 13 s |
| render, 27 steps, 10 s | 79 s (e2e test, process start to exit) | most of it is the fixed cost: import, 25 s of weight loading, text encoding, decode; the 27 steps themselves are about 15 s |
| render, resume (raw.wav present) | 0.06 s | `start` + `done`, nothing imported |
| separate (htdemucs, MPS) | 9.9 s first time (incl. 89 MB download), 3.1 s warm | |
| mix (ffmpeg, two-pass loudnorm, mp3, cover) | 1.6 s | final measured at -13.99 LUFS, -2.75 dBTP |
| convert, missing model | 0.04 s | one `error` line, exit 1, no import |
| convert, garbage .pth | 48 s cold / 2 s warm | base models download 8 s (370 MB), rvc-python import 40 s cold, one `error` line |
| rvc base assets | hubert 1.6 s, rmvpe 0.4 s to load on MPS | both from `models/rvc` |

Output: raw.wav 29.91 s at 48 kHz stereo, peak 1.08 (float, no clipping),
no NaN, no silent second; the vocal stem is loud and clear (RMS 0.19 against
0.11 for the beat). float16 on Metal produced no artefacts on this run; if a
render ever comes out silent, set `OVERNIGHT_ACESTEP_DTYPE=float32` and
`OVERNIGHT_DEVICE=cpu`.

Memory: with ACE-Step loaded the machine sat at 12.6 GB of swap used (the
Mac had other things open); the render still ran at the speeds above. Expect
the first render after a reboot to spend longer in "Loading weights".

No real render came near the 10 minute line, so the 27-step comparison was not
required; the e2e test renders 10 s at 27 steps for a quick check.

## Tests

```
"$OVERNIGHT_DATA/venv/bin/python" -m pip install pytest   # or: uv pip install --python … pytest
cd engine && "$OVERNIGHT_DATA/venv/bin/python" -m pytest -q          # 27 tests, ~4 s
OVERNIGHT_E2E=1 "$OVERNIGHT_DATA/venv/bin/python" -m pytest tests/test_e2e.py -s   # real models
```

Unit tests cover the progress emitter (format, clamping, dedupe, and a
subprocess proof that `print`, raw `os.write(1, …)` and a child process all
land on stderr), cover drawing (a 1024 PNG, determinism per song id, long
titles), mix command building and loudnorm parsing, and `check` against an
empty models folder (valid JSON, exit 0, nothing written). The e2e test
renders, resumes, separates, mixes, and checks that convert against a missing
model is one error line, parsing every stdout line as JSON along the way.

## Voice models

The engine never fetches or trains a voice. `convert` runs whatever `.pth`
the user points it at. Neither rvc-python nor the RVC project ships a demo
voice model, so the convert stage was verified with a missing file (clean
error), a garbage file (clean error after the base models were fetched) and a
direct load of hubert + rmvpe on MPS; a real conversion end to end is untested
until the user supplies a model.
