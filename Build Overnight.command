#!/bin/bash
# Double-click me. I build Overnight.app, put it in Applications and open it.
# After that you never need this file (or a terminal) again, unless you update.
#
# What I do, in order:
#   1. make sure Apple's command line tools are there
#   2. install Rust if it's missing (one time, ~5 min, goes in ~/.cargo)
#   3. npm install
#   4. install uv if it's missing (the Python package manager the engine uses)
#   5. install ffmpeg if it's missing (through Homebrew)
#   6. set up the audio engine: a Python environment with torch, ACE-Step,
#      demucs and rvc in the app's data folder (first time: 5-15 minutes and a
#      few GB of downloads; after that, seconds)
#   7. compile the app (first time 3-6 minutes)
#   8. copy it to /Applications and launch it
#
# Nothing here touches your Claude login. The app talks to the `claude`
# command that is already signed in on this Mac. The model weights themselves
# (the music model, the stem splitter) download on first use inside the app,
# not here, so this script never fetches a voice model of any kind.
#
# Checking only:  BUILD_DRY_RUN=1 "./Build Overnight.command"
# says what it found and what it would install or build, then stops.

# Work from the project folder, even if this file was copied somewhere else
# (the Desktop, say).
cd "$(dirname "$0")" || exit 1
for guess in "$HOME/projects/overnight" "$HOME/overnight"; do
  [ -f package.json ] && [ -d src-tauri ] && break
  cd "$guess" 2>/dev/null || continue
done
if [ ! -f package.json ] || [ ! -d src-tauri ]; then
  echo "Can't find the overnight project folder. Run this file from inside it."
  read -n 1 -s -r -p "Press any key to close."
  exit 1
fi

DRY="${BUILD_DRY_RUN:-}"

# Keep a copy of everything below in build.log, next to this file. If a build
# fails, that file has the details.
exec > >(tee "build.log") 2>&1
echo "Build started $(date)"
[ -n "$DRY" ] && echo "(dry run: checks only, nothing gets installed or built)"

say_step() { printf "\n\033[1m== %s\033[0m\n" "$1"; }
would() { printf "  dry run, would: %s\n" "$1"; }
# The pause exists so a double-clicked Terminal window doesn't vanish before
# the last line can be read. Skipped when there is no keyboard (dry runs from
# another script).
pause() {
  [ -n "$DRY" ] && return
  [ -t 0 ] || return
  read -n 1 -s -r -p "Press any key to close."
}
fail() {
  printf "\n\033[31mStopped: %s\033[0m\n" "$1"
  printf "The details are saved in build.log, in the project folder.\n\n"
  pause
  exit 1
}

# A double-clicked script doesn't get your terminal's PATH. Borrow it. The
# tail end covers the places installers put things when the login shell
# doesn't know about them yet (uv and Rust on a fresh Mac, Homebrew).
SHELL_PATH="$(/bin/zsh -lic 'printf "__P__%s\n" "$PATH"' 2>/dev/null | sed -n 's/^__P__//p' | tail -1)"
[ -n "$SHELL_PATH" ] && export PATH="$SHELL_PATH"
export PATH="$HOME/.cargo/bin:$HOME/.local/bin:$PATH:/opt/homebrew/bin:/usr/local/bin:$HOME/.homebrew/bin"

# Where the app keeps everything that is yours: settings, songs, and the
# Python environment this script sets up. Same folder the app itself uses, so
# the engine is ready the moment it opens.
DATA="$HOME/Library/Application Support/com.overnight.studio"

say_step "1/8  Apple command line tools"
if ! xcode-select -p >/dev/null 2>&1; then
  [ -n "$DRY" ] && fail "Apple's command line tools are not installed (xcode-select --install)."
  xcode-select --install
  fail "Apple's command line tools are installing (a window just opened). When it finishes, double-click this file again."
fi
# Will the compiler actually run? After an Xcode update it refuses to until the
# license is accepted again - and then every Rust crate fails in a wall of red.
cc_check() { echo 'int main(void){return 0;}' | cc -x c - -o "${TMPDIR:-/tmp}/overnight-cc-check" 2>&1; }
if ! CC_OUT="$(cc_check)"; then
  if echo "$CC_OUT" | grep -qi "license"; then
    [ -n "$DRY" ] && fail "Xcode's license needs accepting first (sudo xcodebuild -license)."
    echo "Xcode was updated and won't compile anything until its license is accepted again."
    echo "Apple's own prompt comes next: enter your Mac password (nothing shows while you"
    echo "type), press space to page through it (q jumps to the end), then type: agree"
    echo
    sudo xcodebuild -license || fail "The Xcode license wasn't accepted, so nothing can be compiled yet."
    CC_OUT="$(cc_check)" || fail "The compiler still won't run: $CC_OUT"
  else
    fail "The C compiler won't run: $CC_OUT"
  fi
fi
echo "ok ($(xcode-select -p))"

say_step "2/8  Rust"
if ! command -v cargo >/dev/null 2>&1; then
  if [ -n "$DRY" ]; then
    would "install Rust with rustup (curl https://sh.rustup.rs | sh -s -- -y)"
  else
    echo "Not found - installing (one time)..."
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y || fail "Rust install failed."
    # shellcheck disable=SC1091
    source "$HOME/.cargo/env"
  fi
fi
if command -v cargo >/dev/null 2>&1; then
  echo "$(cargo --version) ($(command -v cargo))"
else
  [ -n "$DRY" ] || fail "Rust still isn't on the PATH."
fi

say_step "3/8  JavaScript packages"
command -v npm >/dev/null 2>&1 || fail "npm not found. Install Node.js 20 or newer (https://nodejs.org) and run me again."
NODE_MAJOR="$(node -p 'process.versions.node.split(".")[0]' 2>/dev/null)"
[ "${NODE_MAJOR:-0}" -ge 18 ] || fail "Node.js $(node -v) is too old. Install Node.js 20 or newer (https://nodejs.org) and run me again."
if [ -n "$DRY" ]; then
  echo "node $(node -v) ($(command -v node))"
  would "npm install --no-audit --no-fund"
else
  npm install --no-audit --no-fund || fail "npm install failed."
  # npm sometimes skips the Mac-specific native pieces of the build tools.
  # If the bundler can't start, wipe and reinstall once from scratch.
  if ! npx --no-install vite --version >/dev/null 2>&1; then
    echo "Build tools didn't install cleanly - reinstalling from scratch..."
    rm -rf node_modules package-lock.json
    npm install --no-audit --no-fund || fail "npm install failed (second try)."
    npx --no-install vite --version >/dev/null 2>&1 || fail "The bundler (vite) still won't start. Node version: $(node -v)"
  fi
  echo "node $(node -v) · vite $(npx --no-install vite --version 2>/dev/null)"
fi

say_step "4/8  uv (sets up the engine's Python)"
# uv makes the Python side reproducible: it fetches its own Python 3.10 for
# the engine, so whatever python3 this Mac happens to have doesn't matter.
if ! command -v uv >/dev/null 2>&1; then
  if [ -n "$DRY" ]; then
    would "install uv (curl -LsSf https://astral.sh/uv/install.sh | sh), lands in ~/.local/bin"
  else
    echo "Not found - installing (one time, goes in ~/.local/bin)..."
    curl -LsSf https://astral.sh/uv/install.sh | sh || fail "uv install failed. Install it by hand (https://docs.astral.sh/uv/) and run me again."
  fi
fi
if command -v uv >/dev/null 2>&1; then
  echo "$(uv --version) ($(command -v uv))"
else
  [ -n "$DRY" ] || fail "uv still isn't on the PATH. Open a new terminal window and run me again."
fi

say_step "5/8  ffmpeg (mixes and masters the songs)"
if command -v ffmpeg >/dev/null 2>&1; then
  echo "$(ffmpeg -version 2>/dev/null | head -n 1 | cut -d' ' -f1-3) ($(command -v ffmpeg))"
elif command -v brew >/dev/null 2>&1; then
  if [ -n "$DRY" ]; then
    would "brew install ffmpeg"
  else
    echo "Not found - installing with Homebrew (a few minutes)..."
    brew install ffmpeg || fail "Homebrew couldn't install ffmpeg. Run 'brew install ffmpeg' yourself, then run me again."
    command -v ffmpeg >/dev/null 2>&1 || fail "ffmpeg still isn't on the PATH after the install."
    echo "installed ($(command -v ffmpeg))"
  fi
else
  # Not fatal: the app builds and runs without it, the mix stage just can't
  # finish. The Setup page in the app says the same thing.
  printf "\033[33mffmpeg isn't installed and neither is Homebrew, so I can't install it for you.\033[0m\n"
  echo "Install Homebrew (https://brew.sh) and run 'brew install ffmpeg', or put an ffmpeg"
  echo "build from https://ffmpeg.org on your PATH. The app will build, but songs can't be"
  echo "mixed until ffmpeg is there."
fi

say_step "6/8  Audio engine (Python environment in the app's data folder)"
echo "venv:    $DATA/venv"
echo "models:  $DATA/models"
# setup.sh talks in the same JSON lines the app reads (one {"stage":"setup",...}
# per line). In a terminal that is noise, so show just the message of each.
setup_lines() {
  while IFS= read -r line; do
    case "$line" in
      '{"stage"'*)
        msg="$(printf '%s' "$line" | sed -n 's/.*"message":"\([^"]*\)".*/\1/p')"
        case "$line" in *'"event":"error"'*) msg="error: ${msg:-$line}" ;; esac
        printf '  %s\n' "${msg:-$line}"
        ;;
      *) printf '%s\n' "$line" ;;
    esac
  done
}
if [ -n "$DRY" ]; then
  if [ -f engine/setup.sh ]; then
    would "bash engine/setup.sh \"$DATA/venv\" \"$DATA/models\""
  else
    echo "  engine/setup.sh is not here; a real run would stop at this step."
  fi
  [ -x "$DATA/venv/bin/python" ] && echo "  an engine venv already exists (setup.sh would update it in place)"
else
  [ -f engine/setup.sh ] || fail "engine/setup.sh is missing. Is this a complete copy of the project?"
  mkdir -p "$DATA/models" || fail "Couldn't create $DATA"
  echo "First time this downloads torch, ACE-Step, demucs and rvc-python: 5-15 minutes."
  # `bash` rather than ./ so a lost executable bit (zip downloads) doesn't matter.
  bash engine/setup.sh "$DATA/venv" "$DATA/models" | setup_lines
  [ "${PIPESTATUS[0]}" -eq 0 ] || fail "The engine setup failed. The last lines above say why; the app's Setup page can retry it later."
  echo "engine ok"
fi

say_step "7/8  Compiling Overnight (first time: 3-6 minutes)"
# The app carries its own copy of the engine source as a bundled resource, so
# a built .app works even if this project folder is later moved or deleted.
# The venv stays in the data folder (it is gigabytes, and machine-specific).
RES="src-tauri/resources/engine"
if [ -n "$DRY" ]; then
  would "copy engine/ to $RES/ (without .venv, __pycache__, tests)"
  would "npm run tauri build -- --bundles app"
else
  [ -d engine ] || fail "The engine/ folder is missing."
  rm -rf "$RES"
  mkdir -p "$RES" || fail "Couldn't create $RES"
  rsync -a \
    --exclude '.venv' --exclude '__pycache__' --exclude 'tests' \
    --exclude '*.pyc' --exclude '.pytest_cache' --exclude '.ruff_cache' --exclude '.mypy_cache' \
    engine/ "$RES/" || fail "Couldn't copy the engine into the app's resources."
  npm run tauri build -- --bundles app || fail "The build failed."
fi

APP="src-tauri/target/release/bundle/macos/Overnight.app"
if [ -n "$DRY" ]; then
  would "install $APP into /Applications and open it"
  printf "\n\033[1mDry run finished.\033[0m Nothing was changed.\n"
  exit 0
fi
[ -d "$APP" ] || fail "Build finished but I can't find the app at: $APP"

say_step "8/8  Installing"
DEST="/Applications"
[ -w "$DEST" ] || { DEST="$HOME/Applications"; mkdir -p "$DEST"; }
osascript -e 'tell application "Overnight" to quit' >/dev/null 2>&1
rm -rf "$DEST/Overnight.app"
cp -R "$APP" "$DEST/" || fail "Couldn't copy the app into $DEST."
touch "$DEST/Overnight.app"   # nudges Finder and the Dock to pick up a new icon

printf "\n\033[1mDone.\033[0m  %s/Overnight.app\n" "$DEST"
command -v claude >/dev/null 2>&1 || printf "\033[33mHeads up:\033[0m the \`claude\` command isn't on this Mac yet. Install Claude Code and sign in, or there is nobody to write the lyrics: https://claude.com/claude-code\n"
echo "The first song also downloads the music model (several GB), so give it a while."
echo "Drag it to your Dock if you want it there. Opening it now..."
open "$DEST/Overnight.app"
echo
pause
