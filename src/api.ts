// Thin wrapper over the Tauri backend. Outside the desktop app (plain
// `npm run dev` in a browser) every call falls back to sample data so the
// pages can be worked on and demoed. Command names, argument names and
// return shapes follow SPEC.md section 4.1.

import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import type { LogEvent, Persona, ProgressEvent, RenderFrom, Settings, SetupStatus, Song, SongPatch } from "./types";
import { defaultPersona, defaultSettings, mockAudioUrl, mockCover, mockInstallLines, mockLyrics, mockSetup, mockSongs, mockTitle } from "./mock";

export const inTauri = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

const params = () => new URLSearchParams(window.location.search);
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

// ---------------------------------------------------------------- mock state

let mockSettings: Settings = structuredClone(defaultSettings);
let mockPersona: Persona = structuredClone(defaultPersona);
let mockLibrary: Song[] = params().has("empty") ? [] : structuredClone(mockSongs);
let mockStatus: SetupStatus = structuredClone(mockSetup);
let mockInstalledOnce = false;
let mockRenderingId: string | null = null;
let mockCancelled = false;
const progressListeners = new Set<(e: ProgressEvent) => void>();
const logListeners = new Set<(e: LogEvent) => void>();

const emit = (e: Omit<ProgressEvent, "at">) => progressListeners.forEach((fn) => fn({ ...e, at: Date.now() }));
const emitLog = (line: string) => logListeners.forEach((fn) => fn({ line, at: Date.now() }));

// -------------------------------------------------------------------- files

/**
 * URL for a file inside a song folder. In the app this goes through the asset
 * protocol; in the browser the mock serves an SVG cover and a synthesised loop.
 */
export function songFileUrl(dataDir: string, song: Song, file: string): string {
  if (inTauri) return convertFileSrc(dataDir + "/songs/" + song.id + "/" + file);
  if (file.endsWith(".png")) return mockCover(song.title, song.mood, song.persona.name, song.id);
  return mockAudioUrl();
}

export async function dataDir(): Promise<string> {
  if (inTauri) return invoke<string>("data_dir");
  return mockSetup.dataDir;
}

/**
 * Shows the data folder in Finder. There is no backend command for this, so
 * it goes straight through the opener plugin (allowed by opener:default).
 */
export async function openDataDir(dir: string): Promise<void> {
  if (inTauri) return revealItemInDir(dir);
  console.log("reveal", dir);
}

// ----------------------------------------------------------------- settings

export async function getSettings(): Promise<Settings> {
  if (inTauri) return invoke<Settings>("get_settings");
  return structuredClone(mockSettings);
}

export async function setSettings(settings: Settings): Promise<Settings> {
  if (inTauri) return invoke<Settings>("set_settings", { settings });
  mockSettings = structuredClone(settings);
  return structuredClone(mockSettings);
}

// ------------------------------------------------------------------ persona

export async function getPersona(): Promise<Persona> {
  if (inTauri) return invoke<Persona>("get_persona");
  return structuredClone(mockPersona);
}

export async function setPersona(persona: Persona): Promise<Persona> {
  if (inTauri) return invoke<Persona>("set_persona", { persona });
  mockPersona = structuredClone(persona);
  return structuredClone(mockPersona);
}

export async function resetPersona(): Promise<Persona> {
  if (inTauri) return invoke<Persona>("reset_persona");
  mockPersona = structuredClone(defaultPersona);
  return structuredClone(mockPersona);
}

// -------------------------------------------------------------------- setup

export async function setupStatus(): Promise<SetupStatus> {
  if (inTauri) return invoke<SetupStatus>("setup_status");
  await sleep(150);
  // ?installing=1 previews the page while setup.sh is running elsewhere.
  if (params().has("installing") && !mockStatus.installing && !mockInstalledOnce) {
    mockStatus = { ...mockStatus, installing: true, venv: false, check: null };
    void mockInstall(true);
  }
  return structuredClone(mockStatus);
}

async function mockInstall(slow: boolean): Promise<void> {
  mockStatus = { ...mockStatus, installing: true };
  for (const line of mockInstallLines) {
    emitLog(line);
    await sleep(slow ? 900 : 300);
  }
  mockStatus = {
    ...mockStatus,
    installing: false,
    venv: true,
    check: { ...(mockSetup.check ?? mockStatus.check!), rvc: true, models: { acestep: true, demucs: true, rvcAssets: true }, problems: [] },
  };
  mockInstalledOnce = true;
}

export async function installEngine(): Promise<void> {
  if (inTauri) return invoke<void>("install_engine");
  if (mockStatus.installing) throw new Error("The engine is already installing.");
  await mockInstall(false);
}

export async function onLog(fn: (e: LogEvent) => void): Promise<() => void> {
  if (inTauri) return listen<LogEvent>("overnight-log", (e) => fn(e.payload));
  logListeners.add(fn);
  return () => logListeners.delete(fn);
}

export async function pickFile(kind: "pth" | "index"): Promise<string> {
  if (inTauri) return invoke<string>("pick_file", { kind });
  await sleep(300);
  return kind === "pth" ? "/Users/you/Library/Application Support/com.overnight.studio/voices/sample/model.pth" : "/Users/you/Library/Application Support/com.overnight.studio/voices/sample/added_IVF256.index";
}

// -------------------------------------------------------------------- songs

export async function listSongs(): Promise<Song[]> {
  if (inTauri) return invoke<Song[]>("list_songs");
  await sleep(120);
  return structuredClone(mockLibrary);
}

export async function getSong(id: string): Promise<Song> {
  if (inTauri) return invoke<Song>("get_song", { id });
  const s = mockLibrary.find((x) => x.id === id);
  if (!s) throw new Error(`No song ${id}`);
  return structuredClone(s);
}

export async function deleteSong(id: string): Promise<void> {
  if (inTauri) return invoke<void>("delete_song", { id });
  mockLibrary = mockLibrary.filter((x) => x.id !== id);
}

export async function revealSong(id: string): Promise<void> {
  if (inTauri) return invoke<void>("reveal_song", { id });
  console.log("reveal", id);
}

export async function updateSong(id: string, patch: SongPatch): Promise<Song> {
  if (inTauri) return invoke<Song>("update_song", { id, patch });
  const s = mockLibrary.find((x) => x.id === id);
  if (!s) throw new Error(`No song ${id}`);
  Object.assign(s, patch);
  return structuredClone(s);
}

const pad = (n: number) => String(n).padStart(2, "0");
function mockId(): string {
  const d = new Date();
  const stamp = `${d.getFullYear()}${pad(d.getMonth() + 1)}${pad(d.getDate())}-${pad(d.getHours())}${pad(d.getMinutes())}${pad(d.getSeconds())}`;
  const alpha = "abcdefghijklmnopqrstuvwxyz0123456789";
  let tail = "";
  for (let i = 0; i < 4; i++) tail += alpha[Math.floor(Math.random() * alpha.length)];
  return `${stamp}-${tail}`;
}

export async function writeSong(theme: string, mood: string, durationSec: number): Promise<Song> {
  if (inTauri) return invoke<Song>("write_song", { theme, mood, durationSec });
  await sleep(1500);
  const keys = ["F minor", "C# minor", "A minor", "G minor", "D minor"];
  const song: Song = {
    id: mockId(),
    title: mockTitle(theme),
    theme,
    mood,
    createdAt: new Date().toISOString(),
    persona: structuredClone(mockPersona),
    lyrics: mockLyrics(theme, mood, durationSec),
    tags: mockPersona.tags + ", " + [mood.toLowerCase() || "late night", "wet streets", "sparse piano", "wide reverb"].join(", "),
    bpm: 80 + Math.floor(Math.random() * 6),
    key: keys[Math.floor(Math.random() * keys.length)],
    durationSec,
    engine: mockSettings.engine,
    seed: Math.floor(Math.random() * 9_000_000),
    steps: mockSettings.steps,
    stages: { lyrics: "done", render: "pending", separate: "pending", convert: mockSettings.convert ? "pending" : "skipped", mix: "pending" },
    error: null,
    files: { raw: null, vocals: null, instrumental: null, converted: null, final: null, mp3: null, cover: null },
    actualDurationSec: null,
    renderSeconds: null,
  };
  mockLibrary.unshift(song);
  return structuredClone(song);
}

/** Walks the stages over about eight seconds, emitting progress like the engine would. */
export async function renderSong(id: string, from: RenderFrom): Promise<Song> {
  if (inTauri) return invoke<Song>("render_song", { id, from });
  if (mockRenderingId) throw new Error("Another song is rendering. Wait for it to finish or cancel it.");
  const s = mockLibrary.find((x) => x.id === id);
  if (!s) throw new Error(`No song ${id}`);
  mockRenderingId = id;
  mockCancelled = false;
  s.error = null;
  s.engine = mockSettings.engine;
  const order: RenderFrom[] = ["render", "separate", "convert", "mix"];
  const todo = order.slice(order.indexOf(from));
  for (const st of order) {
    if (todo.includes(st)) s.stages[st] = st === "convert" && !mockSettings.convert ? "skipped" : "pending";
  }
  const check = () => {
    if (mockCancelled) throw new Error("Cancelled");
  };
  const started = Date.now();
  try {
    for (const stage of todo) {
      if (s.stages[stage] === "skipped") continue;
      s.stages[stage] = "running";
      if (stage === "render") {
        emit({ songId: id, stage, event: "start", message: mockSettings.engine === "fal" ? "Queued at fal.ai" : "Loading ACE-Step" });
        await sleep(600);
        check();
        emit({ songId: id, stage, event: "log", message: `device: mps, ${s.steps} steps, seed ${s.seed}` });
        for (let i = 1; i <= s.steps; i++) {
          await sleep(3400 / s.steps);
          check();
          emit({ songId: id, stage, event: "progress", pct: Math.round((i / s.steps) * 100), message: `Step ${i}/${s.steps}` });
        }
        s.files.raw = "raw.wav";
        s.actualDurationSec = s.durationSec - 1.7;
        s.renderSeconds = (Date.now() - started) / 1000;
        emit({ songId: id, stage, event: "done", message: `raw.wav ${s.actualDurationSec.toFixed(1)} s`, detail: `${s.renderSeconds.toFixed(1)} s` });
      } else if (stage === "separate") {
        emit({ songId: id, stage, event: "start", message: "Loading demucs (htdemucs)" });
        await sleep(500);
        check();
        emit({ songId: id, stage, event: "progress", pct: 50, message: "Separating vocals" });
        await sleep(900);
        check();
        s.files.vocals = "vocals.wav";
        s.files.instrumental = "instrumental.wav";
        emit({ songId: id, stage, event: "done", message: "vocals.wav, instrumental.wav", detail: "1.4 s" });
      } else if (stage === "convert") {
        emit({ songId: id, stage, event: "start", message: "Loading voice model" });
        await sleep(400);
        check();
        emit({ songId: id, stage, event: "progress", pct: 60, message: `rmvpe pitch, ${mockSettings.voiceModel.pitch >= 0 ? "+" : ""}${mockSettings.voiceModel.pitch} st` });
        await sleep(700);
        check();
        s.files.converted = "vocals_converted.wav";
        emit({ songId: id, stage, event: "done", message: "vocals_converted.wav", detail: "1.1 s" });
      } else {
        emit({ songId: id, stage, event: "start", message: "Mixing stems" });
        await sleep(500);
        check();
        emit({ songId: id, stage, event: "progress", pct: 40, message: "loudnorm -14 LUFS" });
        await sleep(600);
        check();
        emit({ songId: id, stage, event: "log", message: "cover.png drawn" });
        await sleep(400);
        check();
        s.files.final = "final.wav";
        s.files.mp3 = "final.mp3";
        s.files.cover = "cover.png";
        emit({ songId: id, stage, event: "done", message: "final.mp3 320k", detail: "1.5 s" });
      }
      s.stages[stage] = "done";
    }
  } catch (e) {
    const running = todo.find((st) => s.stages[st] === "running");
    if (running) {
      s.stages[running] = "error";
      s.error = String((e as Error).message ?? e);
      emit({ songId: id, stage: running, event: "error", message: s.error });
    }
    throw e;
  } finally {
    mockRenderingId = null;
  }
  return structuredClone(s);
}

export async function cancelRender(id: string): Promise<void> {
  if (inTauri) return invoke<void>("cancel_render", { id });
  if (mockRenderingId === id) mockCancelled = true;
}

export async function onProgress(fn: (e: ProgressEvent) => void): Promise<() => void> {
  if (inTauri) return listen<ProgressEvent>("overnight-progress", (e) => fn(e.payload));
  progressListeners.add(fn);
  return () => progressListeners.delete(fn);
}
