# Overnight

A private song studio with exactly one artist in it: a made-up one. You type what the song is about, Claude writes the lyrics, a music model sings them over a beat it also plays, ffmpeg mixes and masters, and a finished mp3 with cover art lands in your library. Moody Toronto rap and R&B by default, though the artist page lets you steer it anywhere. One button, one song, no accounts.

> ## You need these before it will run
>
> | | What | Why |
> |---|---|---|
> | **Required** | A **Mac with Apple Silicon** (M1 or later, macOS 13+), 16 GB or more | The music model runs on the Mac's GPU through Metal. 18 GB is what it was built on; 8 GB will not do. |
> | **Required** | **[Claude Code](https://claude.com/claude-code)** installed and **signed in with your own Claude subscription** (run `claude`, then `/login`) | Claude is the songwriter. The app runs the `claude` command that's already on your Mac. No Claude login, no lyrics. |
> | **Required** | **[Node.js](https://nodejs.org) 20 or newer** | Builds the app. |
> | Auto-installed | Rust, Apple's command line tools, `uv`, ffmpeg (via Homebrew), a Python 3.10 environment with torch, ACE-Step, demucs and rvc-python | The builder installs or prompts for each. The Python side lives in the app's data folder, not in the project. |
> | Downloaded on first use | The ACE-Step music model (several GB), the demucs stem splitter, RVC's shared assets | Inside the app, with progress shown, into `models/` in the data folder. Never during the build. |
> | Optional | A **[fal.ai](https://fal.ai) key** | Renders the song on fal's GPUs instead of your Mac. Faster; costs money per song; your lyrics leave the machine. |
> | Optional | An **RVC voice model** (`.pth`, optionally its `.index`) that you made or have the right to use | Re-sings the vocal in that voice. Without one the song keeps the model's own voice, which is fine. |
>
> **No accounts, keys or logins ship with this project.** Each person's copy uses the Claude login on *their* Mac, in `~/.claude`, and spends *their* plan's usage. The only thing that leaves your machine by default is the lyric-writing conversation with Claude. Everything audio happens on your Mac unless you switch the engine to fal.

## What's original, and what's yours to answer for

Every song Overnight makes is new. Claude writes fresh lyrics under standing rules: never quote or paraphrase an existing song, never name a real person, and the style prompt sent to the music model describes a sound, never an artist. The singing voice is ACE-Step's own model voice, not a recording of anybody.

Voice conversion is optional and strictly bring-your-own. The app runs whatever RVC `.pth` you point it at and **never searches for, downloads, or trains one**. That's a deliberate line, not a missing feature. Train a model on your own voice with any RVC trainer (a few minutes of clean recordings is enough), or use one whose owner said you could. Turning a made-up artist into an imitation of a real one is on you, and it's exactly what this app is not for.

ACE-Step publishes its code and weights under Apache 2.0, demucs and RVC under MIT. Check the licence of anything else you plug in before you publish what comes out.

## Install

1. Get the code: `git clone <this repo>` (or download the zip and unpack it).
2. Double-click **`Build Overnight.command`** in the project folder.
   (First time, macOS may say it can't be opened: right-click it, choose **Open**, then **Open** again. Or in Terminal: `chmod +x "Build Overnight.command" && xattr -c "Build Overnight.command"`.)
3. Go make coffee. The first run installs Rust if it's missing, `npm install`, `uv`, ffmpeg, and then builds the engine's Python environment, which downloads torch, ACE-Step, demucs and rvc-python (5 to 15 minutes, a few GB). Then it compiles the app (3 to 6 minutes), copies **Overnight.app** to Applications and opens it. Runs after the first one take a couple of minutes.

Why does everyone build their own copy instead of downloading an app? Because an app that isn't signed with a paid Apple developer certificate gets blocked by macOS on anyone else's Mac. One you compiled yourself runs without complaint.

The builder checks Apple's command line tools (and walks you through re-accepting the Xcode license if an Xcode update is blocking the compiler), installs what's missing, sets up the engine in `~/Library/Application Support/com.overnight.studio/venv`, copies the engine source into the app bundle, compiles, installs and launches. Everything it prints is saved to `build.log`. There's no name prompt and nothing to configure; the icon is fixed.

To see what it would do without doing it: `BUILD_DRY_RUN=1 "./Build Overnight.command"`. It checks each tool, says what it would install or build, and stops.

By hand, if you prefer:

```bash
npm install
bash engine/setup.sh "$HOME/Library/Application Support/com.overnight.studio/venv" \
                    "$HOME/Library/Application Support/com.overnight.studio/models"
npm run tauri dev                                    # live-reload development (uses ../engine directly)
rsync -a --exclude .venv --exclude __pycache__ --exclude tests engine/ src-tauri/resources/engine/
npm run tauri build -- --bundles app                 # the .app, in src-tauri/target/release/bundle/macos/
```

## First launch

Open **Setup** first. It's a checklist: Claude CLI, `uv`, ffmpeg, the engine environment, torch and which device it will use (`mps` on Apple Silicon), ACE-Step, demucs, rvc, and whether each model's weights are on disk yet. If the builder did its job, everything but the weights is green. **Install engine** re-runs the setup with a live log if anything is red.

Then **Studio**. Type what the song is about ("she saw the message at 2am and never answered"), pick a mood chip or none, pick a length, hit **Write it** (or Cmd+Enter). Claude comes back with a title, a style tag line, and lyrics laid out in sections. Edit anything, or **Rewrite**. **Render** starts the engine. The first render downloads the music model before it does anything else; that is several GB and the progress line says so. Every render after that starts in seconds.

The **Artist** page is the persona: name, tagline, how the voice is described, the standing style tags, the themes, writing notes, rules, and the cover art style. All of it goes into every lyric prompt and every render. Change it and the next song follows. **Reset to default** brings back the shipped one (`src-tauri/resources/default-persona.json`).

## How a song gets made

| Stage | Who | What | Measured (M3 Pro, 18 GB) |
|---|---|---|---|
| 1. Lyrics | `claude -p` | Gets the persona, your theme and mood, the target length mapped to a song structure, and the format rules. Answers with JSON: title, tags, lyrics, bpm, key. Sonnet at medium effort by default. Claude is given no tools at all: no web, no shell, no files. | 20-60 s |
| 2. Render | ACE-Step, on the Mac's GPU (or fal) | Sings the lyrics over a beat it plays, guided by the tag line. 60 steps for quality, 27 for a fast draft. Writes `raw.wav`. | 85 s for a 30 s song at 60 steps on the GPU: about a minute to load the model, then close to real time. A 2:30 song lands in 3 to 4 minutes. The first render also downloads the model (about 7 GB). |
| 3. Separate | demucs (htdemucs) | Splits `raw.wav` into `vocals.wav` and `instrumental.wav`. Also downloads its weights the first time. | 4 s for a 30 s song |
| 4. Convert | rvc-python | Only if voice conversion is on: re-sings `vocals.wav` in your `.pth` voice, writes `vocals_converted.wav`. Skipped otherwise. | not measured here (no voice model on this Mac); expect about a minute on the CPU |
| 5. Mix | ffmpeg | The converted vocal (or the original one) over the instrumental, loudness-normalised to -14 LUFS, true peak -1 dB. Writes `final.wav` and `final.mp3` (320k, tagged, cover embedded) and draws `cover.png` if it isn't there yet. If no stems exist it masters `raw.wav` as is. | 1 to 2 s |

The Studio shows the five stages as a list with a live message and elapsed time each, a percentage for the render, and Cancel. Cancel kills the engine and marks that stage `Cancelled`; **Re-mix** (from stage 5) and **Re-sing** (from stage 4) pick up from the files that already exist, so a cancelled or failed song is never lost. The finished song plays in place, with **Reveal in Finder** for the folder.

The times above are ballpark. Downloads happen once. Renders scale with length and steps; a 1:30 draft at 27 steps is the quick way to try an idea before rendering the 2:30 version at 60.

The cover is drawn by the engine, not by an AI: a dark grainy gradient in the persona's colours, one soft light spot, the title big in a condensed face, the artist name under it, the mood in a corner. It's seeded from the song id, so re-mixing keeps the same cover.

### Where a song lives

Every song is a folder under `songs/` in the data folder, named by its id (`YYYYMMDD-HHMMSS-xxxx`, local time plus four random characters):

```
song.json              the record: title, theme, mood, persona snapshot, lyrics, tags, bpm, key,
                       seed, steps, which engine, stage states, file names, timings
lyrics.txt             the lyrics exactly as sent to the model
raw.wav                what the music model produced (its vocal + the beat)
vocals.wav             separated vocal stem
instrumental.wav       separated beat
vocals_converted.wav   the vocal after voice conversion (only if that stage ran)
final.wav              mixed and mastered
final.mp3              the same, 320k, with title / artist / album / year tags and the cover
cover.png              1024 x 1024
engine.log             everything the engine printed while working on this song
```

Nothing is stored in the project folder, so `git pull` and rebuilding never touch your songs.

## Local or fal

**Local** (the default) runs ACE-Step in the engine's Python environment on your Mac's GPU. Free, private, slow-ish, and it needs the memory: close the big things while it renders. The first render fetches the model weights into `models/`.

**fal** sends the tag line, lyrics, length, steps, guidance and seed to `fal-ai/ace-step` on fal.ai with your key, polls until it's done, and downloads the result to `raw.wav`. Same model, somebody else's GPU, a minute or so per song, billed to your fal account. Everything after the render (separate, convert, mix) still runs locally. Switch in **Setup**, paste the key there; it's saved in `settings.json` on your Mac and nowhere else.

## Voice conversion

Off by default. To turn it on: put your RVC model somewhere sensible (the `voices/` folder in the data folder exists for exactly this, any layout you like), open **Setup**, switch **Voice conversion** on, and pick the `.pth` with the file dialog; add the `.index` if you have one (it helps timbre, it isn't required). The knobs are RVC's own: pitch in semitones (a deep model singing a high part wants a negative number), the pitch method (`rmvpe` is the good default; `harvest`, `crepe` and `pm` are there if you know why), index rate, protect (higher keeps more of the original consonants), filter radius and RMS mix rate. RVC's shared assets (the content encoder and the pitch model) download into `models/` the first time the stage runs.

If the stage errors with the model loading fine but the audio sounding wrong, try pitch first, then a lower index rate.

## Your data and settings

Everything lives in `~/Library/Application Support/com.overnight.studio/`:

- `settings.json` - the knobs below. Missing keys use their defaults.
- `persona.json` - what the Artist page edits
- `songs/` - one folder per song, above
- `venv/` - the engine's Python environment (the builder or Setup makes it; delete it to start over)
- `models/` - model weights (ACE-Step, demucs, RVC assets). The engine's caches point here so nothing lands in `~/.cache`.
- `voices/` - a home for your RVC models
- `workdir/` - the empty folder Claude is run in, so it never sees a project's `CLAUDE.md`
- `engine.log` - the setup / install log

| Key | Default | Meaning |
|---|---|---|
| `engine` | `local` | `local` renders ACE-Step in the venv; `fal` uses fal.ai and needs `falKey` |
| `falKey` | *(empty)* | Your fal.ai key. Only read when `engine` is `fal` |
| `claudeBin` | *(empty = auto-detect)* | Absolute path to `claude`, if the app can't find it |
| `claudeModel` | *(empty = sonnet)* | `opus`, or `default` for whatever your CLI is set to |
| `claudeEffort` | `medium` | `low`, `high`, or `default` |
| `uvBin` | *(empty = auto-detect)* | Absolute path to `uv` |
| `device` | `auto` | `auto` picks `mps` when it's available, else `cpu`. Force `cpu` to compare |
| `steps` | 60 | ACE-Step inference steps. 27 is the fast draft, 60 the quality setting |
| `guidance` | 15.0 | ACE-Step guidance scale. Higher follows the tags harder |
| `defaultDurationSec` | 150 | Length preselected in the Studio (1:30 / 2:30 / 3:30 are 90 / 150 / 210) |
| `convert` | false | Run the voice-conversion stage. Needs `voiceModel.pth` |
| `voiceModel.pth`, `voiceModel.index` | *(empty)* | Absolute paths to your RVC model and its optional index |
| `voiceModel.pitch` | 0 | Semitones up or down |
| `voiceModel.method` | `rmvpe` | Pitch extraction: `rmvpe`, `harvest`, `crepe`, `pm` |
| `voiceModel.indexRate` | 0.66 | How much the index steers timbre, 0 to 1 |
| `voiceModel.protect` | 0.33 | Guards voiceless consonants and breaths, 0 to 0.5 |
| `voiceModel.filterRadius` | 3 | Median filter on the pitch curve |
| `voiceModel.rmsMixRate` | 0.25 | How much of the original loudness envelope to keep |
| `vocalsGainDb` | 0.0 | Vocal level relative to the beat in the mix |
| `mp3Bitrate` | 320 | kbps for `final.mp3` |

## Troubleshooting

- **The builder "can't be opened because it is from an unidentified developer."** Right-click, Open. Or `xattr -c "Build Overnight.command"`.
- **Every Rust crate fails to compile after an Xcode update.** The builder notices the licence message and runs `sudo xcodebuild -license` for you. If you're building by hand, do that yourself.
- **Setup says Claude CLI is missing.** Install [Claude Code](https://claude.com/claude-code), run `claude` once and `/login`. If it's installed somewhere unusual, put the absolute path in `claudeBin`.
- **The engine setup failed.** `engine.log` in the data folder has the whole install. **Install engine** in Setup retries; if it keeps failing, delete `venv/` in the data folder and run the builder again. If ACE-Step and rvc-python refuse to share one environment (torch version pins), setup creates a second one, `venv-rvc`, just for conversion; Setup's checklist says when that happened.
- **The first render sits at "Loading ACE-Step" for ages.** It's downloading the weights (several GB) into `models/`. The log line under the stage shows progress. Once.
- **Render is out of memory, or takes forever.** Close the browser with 40 tabs. Drop `steps` to 27, pick the 1:30 length, or switch the engine to fal for that song. `device: cpu` works but is very slow.
- **The song came back mumbled, or the model sang the section tags.** Check the lyrics: section tags go on their own lines in lower case square brackets (`[verse]`, `[chorus]`), plain lines under them, no chords, no timestamps, no parentheses. **Rewrite** gets a fresh take; **Render** again gets a new seed.
- **Mix fails with "ffmpeg not found".** `brew install ffmpeg`, then **Re-mix**.
- **Voice conversion errors immediately.** Pick the `.pth` in Setup (the path is absolute, so a moved file needs re-picking). The first run downloads RVC's shared assets; a failed download leaves a half file, so delete `models/` entries it mentions and retry.
- **fal says unauthorized or out of credit.** The key in Setup, or your fal balance.
- **A song is stuck "running" after a crash.** Open it in the Library and use Re-mix or Re-sing; the stages re-check what's on disk and continue from there. `delete_song` (the trash button, click twice) removes the folder.
- **Where are the logs?** `build.log` next to the builder; `engine.log` in the data folder for setup; `songs/<id>/engine.log` for each song.

## Where things live

```
Build Overnight.command        the double-click builder (BUILD_DRY_RUN=1 for checks only)
tools/make-icon.py             draws the app icon (PNG sizes, .icns via iconutil, .ico); outputs are committed
engine/                        the audio pipeline, Python 3.10+, run inside the venv
  overnight_engine/            check, render, separate, convert, mix, run; progress as JSON lines
  setup.sh                     creates the venv with uv and installs everything; never downloads weights
  pyproject.toml
src/                           React front end
  App.tsx                      shell: nav, routing
  api.ts                       every command, with a mock fallback so `npm run dev` works in a browser
  pages: Studio, Library, Artist, Setup
src-tauri/src/
  lib.rs                       commands, events, state (one render at a time)
  model.rs                     Settings, Persona, Song and their defaults
  store.rs                     the data folder, atomic writes, settings read-modify-write
  paths.rs                     finds `claude`, `uv`, `ffmpeg` when launched from Finder
  lyrics.rs                    the Claude runner and the lyric prompt
  engine.rs                    runs the Python engine, parses its progress; the fal renderer
src-tauri/resources/
  default-persona.json         the shipped artist
  engine/                      a copy of engine/ made by the builder (bundled into the .app; in dev the app uses ../engine)
src-tauri/icons/               the app icon, generated by tools/make-icon.py
SPEC.md                        the build spec: file layout, JSON shapes, the engine protocol
```

UI only, no backend, fake songs and a fake render that walks the five stages in about eight seconds: `npm run dev`, open the address Vite prints.

## Tests

```bash
cd src-tauri && cargo test                              # lyric JSON parsing, song ids, settings defaults,
                                                        # progress-line parsing, fal response parsing
cd engine && uv run pytest                              # progress emitter, cover drawing, mix arguments,
                                                        # `check` on a bare venv (no models needed)
OVERNIGHT_E2E=1 uv run pytest                           # the real thing: a 30 s render, separate, mix, convert
npx tsc --noEmit                                        # the front end type-checks
bash -n "Build Overnight.command" && shellcheck "Build Overnight.command"
BUILD_DRY_RUN=1 "./Build Overnight.command"             # the builder's checks, nothing installed
python3 tools/make-icon.py && iconutil -c iconset src-tauri/icons/icon.icns -o /tmp/on.iconset
```

## Credits

Music: [ACE-Step](https://github.com/ace-step/ACE-Step) by ACE Studio and StepFun (Apache 2.0). Stems: [Demucs](https://github.com/facebookresearch/demucs) by Meta (MIT). Voice conversion: [RVC](https://github.com/RVC-Project/Retrieval-based-Voice-Conversion-WebUI) through [rvc-python](https://github.com/daswer123/rvc-python) (MIT). Mixing: [ffmpeg](https://ffmpeg.org). Python environments: [uv](https://docs.astral.sh/uv/). Covers: Pillow. Built with [Tauri](https://tauri.app). The app icon is drawn by `tools/make-icon.py` with DIN Condensed, a font that ships with macOS and is not bundled. Not affiliated with Anthropic, ACE Studio, StepFun, Meta, fal.ai, or any artist, living or otherwise.
