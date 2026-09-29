# Overnight — build spec

A private Mac app that makes original songs in the moody Toronto rap / R&B lane.
Claude writes the lyrics. A music model sings them. Optionally a voice-conversion
model (any RVC `.pth` the user supplies) re-sings the vocal. ffmpeg mixes and
masters. One button, one song, a library to play them from.

Same house style as `~/projects/my-daily-newspaper`: Vite + React + TypeScript,
Tauri v2, Rust backend, a double-click builder script, everything the user owns
in the app data folder, no accounts or keys required for the default path.

## 1. Where things live

Repo:

```
engine/                      Python package: the audio pipeline (see §3)
  overnight_engine/          the package
  setup.sh                   creates the venv and installs deps (see §3.6)
  pyproject.toml
src/                         React front end (see §5)
src-tauri/                   Rust backend (see §4)
  resources/default-persona.json
Build Overnight.command      the builder (see §6)
README.md
```

App data folder (macOS): `~/Library/Application Support/com.overnight.studio/`

```
settings.json                Settings (§2.1)
persona.json                 Persona (§2.2)
venv/                        Python virtualenv the engine runs in (created by setup.sh)
models/                      model weights (ACE-Step checkpoints, demucs, rvc assets) — HF/ace caches point here
voices/                      where the user drops RVC models (any layout; they pick the .pth in Setup)
songs/<id>/                  one folder per song (§2.3)
  song.json                  Song record
  lyrics.txt                 the lyrics as sent to the model
  raw.wav                    what the music model produced (model vocal + beat)
  vocals.wav                 separated vocal stem
  instrumental.wav           separated beat (everything but vocals)
  vocals_converted.wav       vocal after voice conversion
  final.wav                  mixed and mastered
  final.mp3                  same, mp3 320k with cover + tags
  cover.png                  1024x1024 cover art
  engine.log                 everything the engine printed for this song
workdir/                     empty scratch folder Claude runs in
engine.log                   setup / install log
```

The engine source is bundled into the .app as a Tauri resource (`src-tauri/resources/engine/`
copied from `engine/` at build time by the builder; in dev the backend falls back to
`../engine` relative to the repo). The venv is NOT bundled; `setup.sh` creates it in the
data folder.

Song id: `YYYYMMDD-HHMMSS-xxxx` (local time + 4 random lowercase alphanumerics).

## 2. Data types (Rust `model.rs` ⇄ TypeScript `types.ts`; JSON is camelCase)

### 2.1 Settings

```jsonc
{
  "engine": "local",              // "local" (ACE-Step in the venv) | "fal" (fal-ai/ace-step, needs falKey)
  "falKey": "",
  "claudeBin": "",                // empty = auto-detect (paths.rs)
  "claudeModel": "",              // empty = "sonnet"; "opus"; "default" = CLI default
  "claudeEffort": "medium",       // low | medium | high | default
  "uvBin": "",                    // empty = auto-detect
  "device": "auto",               // auto | mps | cpu   (auto = mps if available)
  "steps": 60,                    // ACE-Step inference steps (27 fast, 60 quality)
  "guidance": 15.0,
  "defaultDurationSec": 150,
  "convert": false,               // run the voice-conversion stage (needs voiceModel.pth)
  "voiceModel": {
    "pth": "", "index": "",       // absolute paths; empty = none
    "pitch": 0,                   // semitones
    "method": "rmvpe",            // rmvpe | harvest | crepe | pm
    "indexRate": 0.66,
    "protect": 0.33,
    "filterRadius": 3,
    "rmsMixRate": 0.25
  },
  "vocalsGainDb": 0.0,            // vocal level relative to the beat in the mix
  "mp3Bitrate": 320
}
```

Every field has a default; missing keys use defaults (serde `#[serde(default)]`, same as the
newspaper). Read-modify-write on save.

### 2.2 Persona

```jsonc
{
  "name": "Overnight",
  "tagline": "songs for the drive home",
  "voice": "deep baritone, half-sung melodic rap, light autotune, laid-back and conversational, wounded but confident",
  "tags": "melodic rap, r&b, trap soul, moody, atmospheric pads, sparse 808s, soft hi-hats, late night, rainy city, male vocal, autotune, 82 bpm, minor key",
  "themes": "late nights, old flames, loyalty and betrayal, success that feels lonely, the city at 3am, texts left on read",
  "voiceNotes": "first person, conversational, specific details (street names, times, brands), flexes that turn into confessions, hooks that repeat one plain line",
  "rules": "original lyrics only; never quote or paraphrase an existing song; no real people's names",
  "coverStyle": "dark, grainy, a single cold light source, blue and amber"
}
```

Shipped default in `src-tauri/resources/default-persona.json`. The user edits it on the
Artist page. `reset_persona` restores the default.

### 2.3 Song

```jsonc
{
  "id": "20260929-153000-k3q9",
  "title": "Left On Read",
  "theme": "she saw the message at 2am and never answered",   // what the user typed
  "mood": "late night drive",                                   // chip the user picked, may be ""
  "createdAt": "2026-09-29T15:30:00-07:00",
  "persona": { ...snapshot of Persona at write time... },
  "lyrics": "[verse]\n...\n\n[chorus]\n...",                    // ACE-Step format, see §3.2
  "tags": "melodic rap, r&b, ...",                              // style prompt sent to the model
  "bpm": 82,
  "key": "F minor",
  "durationSec": 150,
  "engine": "local",                                            // engine used for render
  "seed": 1234567,
  "steps": 60,
  "stages": {                                                   // each: pending | running | done | error | skipped
    "lyrics": "done", "render": "pending", "separate": "pending", "convert": "skipped", "mix": "pending"
  },
  "error": null,                                                // last error message, if any
  "files": { "raw": null, "vocals": null, "instrumental": null, "converted": null, "final": null, "mp3": null, "cover": null },
                                                                // file names relative to the song folder once they exist
  "actualDurationSec": null,
  "renderSeconds": null                                         // wall clock of the render stage
}
```

## 3. The engine (`engine/overnight_engine`, Python ≥ 3.10 in the venv)

Invoked by the Rust backend as `<venv>/bin/python -m overnight_engine <command> ...`
with `cwd` = the song folder's parent and these env vars always set:

- `OVERNIGHT_DATA` = the data folder
- `OVERNIGHT_MODELS` = `<data>/models` (also exported as `HF_HOME`, `TORCH_HOME`, `ACE_STEP_CHECKPOINTS` or whatever the libraries honour; ACE-Step's checkpoint dir must be under here)
- `OVERNIGHT_DEVICE` = `mps` | `cpu` (Rust resolves `auto`)
- `PYTORCH_ENABLE_MPS_FALLBACK=1`

### 3.1 Commands

| Command | Does | Reads | Writes |
|---|---|---|---|
| `check` | Reports what is installed and present. Never downloads. Exit 0 even if things are missing. | — | JSON to stdout (§3.4) |
| `render --dir <songdir>` | Song from `song.json` (`tags`, `lyrics`, `durationSec`, `steps`, `seed`, guidance from settings passed as `--guidance`) via ACE-Step. | song.json | raw.wav |
| `separate --dir <songdir>` | Demucs (htdemucs) two-way split. | raw.wav | vocals.wav, instrumental.wav |
| `convert --dir <songdir> --pth <path> [--index <path>] [--pitch N] [--method rmvpe] [--index-rate F] [--protect F] [--filter-radius N] [--rms-mix-rate F]` | RVC voice conversion of the vocal stem. | vocals.wav | vocals_converted.wav |
| `mix --dir <songdir> [--vocals-gain-db F] [--bitrate 320]` | Mix `vocals_converted.wav` if it exists else `vocals.wav` over `instrumental.wav`; if neither stem exists, master `raw.wav` as is. Loudness-normalise (ffmpeg `loudnorm`, target -14 LUFS, true peak -1 dB). Write final.wav and final.mp3 with ID3 title/artist(persona name)/album("Overnight")/year and the cover embedded. Draw `cover.png` (§3.5) if missing. | stems / raw.wav, song.json | final.wav, final.mp3, cover.png |
| `run --dir <songdir> [--from render\|separate\|convert\|mix] [--no-convert] [+ convert/mix flags]` | The stages in order from `--from`. `--no-convert` skips convert. | | |

Exit code 0 on success; non-zero on failure with a final `error` event (§3.3). `render`
must be resumable: if `raw.wav` already exists and `--force` is not given, skip.

### 3.2 Lyrics and tags format (what ACE-Step expects)

- Structure tags on their own lines, lower case in square brackets: `[intro]`, `[verse]`,
  `[pre-chorus]`, `[chorus]`, `[bridge]`, `[outro]`. Blank line between sections.
- Plain lines of lyrics under each tag. No chords, no timestamps, no ad-lib parentheses
  (the model sings everything it sees; put ad-libs as their own short lines or leave them out).
- `tags` is a comma-separated style description: genre, mood, instruments, vocal descriptor,
  bpm, key. Never an artist's name.
- Instrumental: lyrics `[inst]`.

### 3.3 Progress protocol (stdout, one JSON object per line)

```jsonc
{"stage":"render","event":"start","message":"Loading ACE-Step","at":1727650000123}
{"stage":"render","event":"progress","pct":42,"message":"Step 25/60","at":...}
{"stage":"render","event":"done","message":"raw.wav 148.3 s","detail":"38.1 s","at":...}
{"stage":"separate","event":"error","message":"...human readable...","detail":"...traceback tail...","at":...}
```

`stage` ∈ render | separate | convert | mix | setup. `event` ∈ start | progress | done | error |
log. `pct` 0–100 on progress. Anything the underlying libraries print to stdout must not
break this: redirect their stdout to stderr (or to `engine.log`) inside the engine. Stderr is
captured by Rust into `engine.log`. Also emit `{"stage":"...","event":"log","message":"..."}`
lines for anything worth showing live (download progress, device chosen).

### 3.4 `check` output

```jsonc
{
  "python": "3.10.14",
  "torch": "2.5.1", "device": "mps",           // device actually usable
  "acestep": true, "demucs": true, "rvc": true, "ffmpeg": "/opt/homebrew/bin/ffmpeg",
  "models": { "acestep": true, "demucs": false, "rvcAssets": false },   // weights present on disk
  "problems": ["..."]                          // human readable, may be empty
}
```

### 3.5 Cover art (Pillow, no AI)

1024×1024 PNG: a dark, grainy gradient in the persona's `coverStyle` colours (parse a couple of
colour words; default blue/amber), a single soft light spot, film grain, the title set large in a
condensed sans (bundled `.ttf` in the package), the persona name small under it, the mood in a
corner. Deterministic from the song id (seeded), so re-mixing keeps the same cover.

### 3.6 `setup.sh <venv-dir> <models-dir>`

Idempotent. Uses `uv` (auto-detect; the builder installs it). Steps, each printing a
`{"stage":"setup","event":"log",...}` line: `uv venv --python 3.10 <venv>`; install torch /
torchaudio (Apple Silicon wheels from PyPI), ACE-Step (from its GitHub repo; pin a commit or
tag that is known to run on MPS with `bf16=false`), demucs, rvc-python, Pillow, soundfile,
numpy; then `python -m overnight_engine check`. If ACE-Step and rvc-python cannot share one
environment (torch pins), create a second venv `<venv>-rvc` for rvc-python only and have the
engine's `convert` stage call into it; document it in `check`. Never download model weights
here (they download on first use, with progress events).

The whole thing must be exercised for real on this Mac (M3 Pro, 18 GB, macOS 26): a 30-second
render through `render`, then `separate`, `mix`, and `convert` against a small RVC model
(any freely available test model, or skip convert with a clear note if no model is on hand
— but the code path must at least load rvc-python and fail gracefully). Report wall-clock
times in the README.

## 4. Backend (Rust, `src-tauri/src`)

Modules: `model.rs` (types + defaults), `store.rs` (data dir, settings, persona, songs, atomic
writes), `paths.rs` (already present: login-shell PATH, `find_bin`, `expand_tilde`),
`lyrics.rs` (Claude runner), `engine.rs` (python engine runner + fal renderer), `lib.rs`
(commands, events, state).

### 4.1 Commands

| Command | Args | Returns |
|---|---|---|
| `get_settings` | | Settings |
| `set_settings` | settings: Settings | Settings (as saved) |
| `get_persona` / `set_persona` / `reset_persona` | persona | Persona |
| `setup_status` | | SetupStatus (§4.3) |
| `install_engine` | | Result<(), String>; streams `overnight-log` events; runs `setup.sh` |
| `write_song` | theme, mood, durationSec | Song (lyrics stage done) |
| `update_song` | id, patch {title?, lyrics?, tags?, durationSec?} | Song |
| `render_song` | id, from: "render"\|"separate"\|"convert"\|"mix" | Song; streams `overnight-progress` events; refuses if another render is running |
| `cancel_render` | id | () |
| `list_songs` | | Song[] newest first |
| `get_song` | id | Song |
| `delete_song` | id | () |
| `reveal_song` | id | () — opens the song folder in Finder (opener plugin) |
| `pick_file` | kind: "pth"\|"index" | String path or "" (dialog plugin) |
| `data_dir` | | String |

Events: `overnight-progress` `{songId, stage, event, pct?, message, detail?, at}`;
`overnight-log` `{line, at}` (install output).

### 4.2 Lyrics (`lyrics.rs`)

Run `claude -p` exactly like the newspaper's `editor.rs` (stream-json, stdin prompt, PATH from
`paths::shell_env`, `--strict-mcp-config`, `--disallowedTools Bash,Edit,Write,NotebookEdit`,
`--allowedTools ""` — no web, this is pure writing; `--max-turns 4`; model/effort from
settings; retry once with minimal flags on "unknown option"). Timeout 240 s.

Prompt: the persona (all fields), the theme and mood, target duration (map to structure: ≤90 s →
verse/chorus/verse/chorus; 120–180 s → intro/verse/chorus/verse/chorus/bridge/chorus; ≥210 s →
add a third verse), and the format rules of §3.2. Ask for melodic-rap density (8–12 syllables a
bar, 12–16 bars a verse, 4–8 bar hooks that repeat one plain line), concrete specifics, a hook
that lands on the theme, no real people, no existing lyrics. Output JSON only:
`{"title","tags","lyrics","bpm","key","durationSec","coverMood"}` — `tags` must start from the
persona's tags and add 3–6 song-specific descriptors; never an artist name. Parse leniently
(reuse the newspaper's `extract_json`/`repair_json` approach). Save `lyrics.txt` and `song.json`.

### 4.3 SetupStatus

```jsonc
{
  "dataDir": "...",
  "claude": "/path/or/null", "uv": "...", "ffmpeg": "...",
  "engineSource": "...path to engine dir the app will use...",
  "venv": true,
  "check": { ...§3.4 or null if no venv... },
  "installing": false
}
```

### 4.4 Engine runner (`engine.rs`)

- Locate the engine source: bundled resource dir first, else `../engine` from the executable's
  ancestors (dev), else error with a clear message.
- Spawn `<venv>/bin/python -m overnight_engine <cmd>` with the env of §3, `PYTHONUNBUFFERED=1`,
  stdout parsed line by line as §3.3 (non-JSON lines go to `engine.log`), stderr appended to
  `<songdir>/engine.log`. Update `song.json` stage statuses as events arrive. `kill_on_drop`.
  Cancel = kill the child, mark the running stage `error: "Cancelled"`.
- `engine: "fal"`: `render` is done in Rust instead: POST `https://queue.fal.run/fal-ai/ace-step`
  with header `Authorization: Key <falKey>` and body `{tags, lyrics, duration, number_of_steps,
  guidance_scale, seed}`; poll `.../requests/<id>/status` every 2 s; fetch `.../requests/<id>`;
  download `audio.url` to `raw.wav`; emit the same progress events. Then `separate/convert/mix`
  run in the venv as usual.
- Only one render at a time (AtomicBool in state).

## 5. Front end (React, `src/`)

`api.ts` wraps every command with an `inTauri` check and a mock fallback (fake songs, a fake
progress timeline that walks the stages over ~8 s) so `npm run dev` works in a plain browser.
Audio and covers are loaded with `convertFileSrc(<dataDir>/songs/<id>/<file>)` from
`@tauri-apps/api/core` (asset protocol is enabled in `tauri.conf.json`).

Pages (top nav: Studio · Library · Artist · Setup):

- **Studio.** "What's this one about?" textarea; mood chips (Late night drive, Heartbreak,
  Flex, The city, Loyalty, Nostalgia, Custom); length (1:30 / 2:30 / 3:30); big button
  "Write it". Then the lyrics review: title (editable), tags (editable), lyrics (editable
  textarea, monospace, section tags highlighted), bpm/key, buttons "Rewrite" and "Render".
  Then the render: the five stages as a vertical list with state icons, a live message line
  and elapsed time per stage, a percentage bar for render, Cancel. When done: the player
  (cover, title, persona, waveform-ish progress bar or plain `<audio controls>`), "Reveal in
  Finder", "Re-mix" (from mix), "Re-sing" (from convert), "New song".
- **Library.** Grid of covers with title, date, length, engine; click to play in a bottom player
  bar; delete with two-click confirm (no native confirm()); shows songs mid-render with their
  stage.
- **Artist.** The Persona fields as a form; "Reset to default"; a live preview of the tags string.
- **Setup.** The SetupStatus as a checklist (Claude CLI, uv, ffmpeg, engine venv, torch + device,
  ACE-Step, demucs, rvc, model weights present); "Install engine" with a live log; engine
  choice (local / fal + key field); render quality (steps, guidance); voice conversion: on/off,
  pick .pth / .index (native dialogs via `pick_file`), pitch, method, index rate, protect;
  vocal gain; "Open data folder".

Look: dark studio. Near-black background, warm off-white text, one amber accent, a cold blue
for progress. Condensed sans for titles, tabular numerals. No glassmorphism, no gradients on
buttons. Keyboard: Cmd+Enter submits the theme.

## 6. Builder — `Build Overnight.command`

Same shape as the newspaper's builder: Apple CLT check (and the Xcode-licence recovery),
Rust, Node ≥ 18 + `npm install`, then: install `uv` if missing (`curl -LsSf
https://astral.sh/uv/install.sh | sh`), install ffmpeg via Homebrew if missing (or say how),
run `engine/setup.sh "<data>/venv" "<data>/models"`, copy `engine/` into
`src-tauri/resources/engine/` (minus `.venv`, caches), `npm run tauri build -- --bundles app`,
copy to /Applications, open. Log to `build.log`. No name prompt.

## 7. Tests

- Rust: unit tests for the lyrics JSON parsing, song id format, settings defaults, progress
  line parsing, fal response parsing. `cargo test` must pass.
- Python: `pytest` for the progress emitter, cover drawing, mix argument building, `check` on a
  bare venv. Real-model runs are opt-in (`OVERNIGHT_E2E=1`).
- Front end: `tsc --noEmit` clean; the mock mode renders every page.

## 8. Not in scope for v1

Stems editor, multiple personas, sharing/export to streaming services, any cloning of a real
person's voice by the app itself (the app runs whatever `.pth` the user points it at and never
fetches or trains one).
