// Shapes shared with the Rust backend (model.rs). JSON is camelCase on both
// sides, so these mirror SPEC.md section 2 field for field.

export type Engine = "local" | "fal";
export type Device = "auto" | "mps" | "cpu";
export type ClaudeEffort = "low" | "medium" | "high" | "default";
export type PitchMethod = "rmvpe" | "harvest" | "crepe" | "pm";

export interface VoiceModel {
  /** Absolute paths; empty means none picked. */
  pth: string;
  index: string;
  /** Semitones. */
  pitch: number;
  method: PitchMethod;
  indexRate: number;
  protect: number;
  filterRadius: number;
  rmsMixRate: number;
}

export interface Settings {
  engine: Engine;
  falKey: string;
  claudeBin: string;
  claudeModel: string;
  claudeEffort: ClaudeEffort;
  uvBin: string;
  device: Device;
  steps: number;
  guidance: number;
  defaultDurationSec: number;
  convert: boolean;
  voiceModel: VoiceModel;
  vocalsGainDb: number;
  mp3Bitrate: number;
}

export interface Persona {
  name: string;
  tagline: string;
  voice: string;
  tags: string;
  themes: string;
  voiceNotes: string;
  rules: string;
  coverStyle: string;
}

export type StageName = "lyrics" | "render" | "separate" | "convert" | "mix";
export type StageState = "pending" | "running" | "done" | "error" | "skipped";
export type Stages = Record<StageName, StageState>;

/** Where a re-run starts. Lyrics are never re-run through render_song. */
export type RenderFrom = "render" | "separate" | "convert" | "mix";

export interface SongFiles {
  raw: string | null;
  vocals: string | null;
  instrumental: string | null;
  converted: string | null;
  final: string | null;
  mp3: string | null;
  cover: string | null;
}

export interface Song {
  id: string;
  title: string;
  theme: string;
  mood: string;
  createdAt: string;
  persona: Persona;
  lyrics: string;
  tags: string;
  bpm: number;
  key: string;
  durationSec: number;
  engine: Engine;
  seed: number;
  steps: number;
  stages: Stages;
  error: string | null;
  files: SongFiles;
  actualDurationSec: number | null;
  renderSeconds: number | null;
}

export interface SongPatch {
  title?: string;
  lyrics?: string;
  tags?: string;
  durationSec?: number;
}

/** What `python -m overnight_engine check` reports (SPEC 3.4). */
export interface EngineCheck {
  python: string | null;
  torch: string | null;
  device: string | null;
  acestep: boolean;
  demucs: boolean;
  rvc: boolean;
  ffmpeg: string | null;
  models: { acestep: boolean; demucs: boolean; rvcAssets: boolean };
  problems: string[];
}

export interface SetupStatus {
  dataDir: string;
  claude: string | null;
  uv: string | null;
  ffmpeg: string | null;
  engineSource: string | null;
  venv: boolean;
  check: EngineCheck | null;
  installing: boolean;
}

export type ProgressStage = "render" | "separate" | "convert" | "mix" | "setup";
export type ProgressKind = "start" | "progress" | "done" | "error" | "log";

/** Payload of the `overnight-progress` event. */
export interface ProgressEvent {
  songId: string;
  stage: ProgressStage;
  event: ProgressKind;
  pct?: number;
  message: string;
  detail?: string;
  at: number;
}

/** Payload of the `overnight-log` event (install output). */
export interface LogEvent {
  line: string;
  at: number;
}
