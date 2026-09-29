// Sample data for running the app in a plain browser (`npm run dev`) without
// the Tauri backend. Nothing here is real: the covers are drawn as SVG, the
// lyrics come from a template, the audio is a synthesised 808 loop.

import type { EngineCheck, Persona, Settings, SetupStatus, Song, Stages } from "./types";
import { seeded } from "./util";

export const defaultSettings: Settings = {
  engine: "local",
  falKey: "",
  claudeBin: "",
  claudeModel: "",
  claudeEffort: "medium",
  uvBin: "",
  device: "auto",
  steps: 60,
  guidance: 15.0,
  defaultDurationSec: 150,
  convert: false,
  voiceModel: {
    pth: "",
    index: "",
    pitch: 0,
    method: "rmvpe",
    indexRate: 0.66,
    protect: 0.33,
    filterRadius: 3,
    rmsMixRate: 0.25,
  },
  vocalsGainDb: 0.0,
  mp3Bitrate: 320,
};

export const defaultPersona: Persona = {
  name: "Overnight",
  tagline: "songs for the drive home",
  voice: "deep baritone, half-sung melodic rap, light autotune, laid-back and conversational, wounded but confident",
  tags: "melodic rap, r&b, trap soul, moody, atmospheric pads, sparse 808s, soft hi-hats, late night, rainy city, male vocal, autotune, 82 bpm, minor key",
  themes: "late nights, old flames, loyalty and betrayal, success that feels lonely, the city at 3am, texts left on read",
  voiceNotes: "first person, conversational, specific details (street names, times, brands), flexes that turn into confessions, hooks that repeat one plain line",
  rules: "original lyrics only; never quote or paraphrase an existing song; no real people's names",
  coverStyle: "dark, grainy, a single cold light source, blue and amber",
};

// ------------------------------------------------------------------ covers

/** A stand-in for cover.png: dark gradient, one cold light, the title set big. */
export function mockCover(title: string, mood: string, persona: string, id: string): string {
  const r = seeded(id);
  const cx = Math.round(200 + r() * 600);
  const cy = Math.round(150 + r() * 400);
  const hue = Math.round(200 + r() * 30);
  const esc = (s: string) => s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/"/g, "&quot;");
  const words = title.split(" ");
  const lines: string[] = [];
  let cur = "";
  for (const w of words) {
    if ((cur + " " + w).trim().length > 12 && cur) {
      lines.push(cur);
      cur = w;
    } else cur = (cur + " " + w).trim();
  }
  if (cur) lines.push(cur);
  // Lay the title out from the bottom up so a long one grows upward instead
  // of running off the canvas.
  const size = lines.length > 2 ? 120 : 150;
  const lastY = 860;
  const text = lines
    .map((l, i) => `<text x='72' y='${lastY - (lines.length - 1 - i) * size * 0.92}' font-size='${size}'>${esc(l.toUpperCase())}</text>`)
    .join("");
  const svg = `<svg xmlns='http://www.w3.org/2000/svg' width='1024' height='1024' viewBox='0 0 1024 1024'>
  <defs>
    <radialGradient id='g' cx='${cx}' cy='${cy}' r='620' gradientUnits='userSpaceOnUse'>
      <stop offset='0' stop-color='hsl(${hue} 60% 62%)' stop-opacity='.85'/>
      <stop offset='.35' stop-color='hsl(${hue} 55% 30%)' stop-opacity='.55'/>
      <stop offset='1' stop-color='#07070a' stop-opacity='0'/>
    </radialGradient>
    <filter id='n'><feTurbulence type='fractalNoise' baseFrequency='.9' numOctaves='2' stitchTiles='stitch'/><feColorMatrix values='0 0 0 0 0.5 0 0 0 0 0.5 0 0 0 0 0.5 0 0 0 .18 0'/></filter>
  </defs>
  <rect width='1024' height='1024' fill='#0a0a0e'/>
  <rect width='1024' height='1024' fill='url(#g)'/>
  <rect width='1024' height='1024' filter='url(#n)' opacity='.6'/>
  <g font-family='Avenir Next Condensed, Helvetica Neue, Arial Narrow, sans-serif' font-weight='700' fill='#f2efe9'>${text}</g>
  <text x='72' y='${lastY + 56}' font-family='Helvetica Neue, Arial, sans-serif' font-size='30' letter-spacing='6' fill='#e8a33d'>${esc(persona.toUpperCase())}</text>
  <text x='952' y='96' text-anchor='end' font-family='Helvetica Neue, Arial, sans-serif' font-size='26' letter-spacing='4' fill='#8b8f9a'>${esc(mood.toUpperCase())}</text>
</svg>`;
  return "data:image/svg+xml;utf8," + encodeURIComponent(svg);
}

// ------------------------------------------------------------------- audio

let audioUrl: string | null = null;

/** Eight seconds of a soft 808 loop at 82 bpm, made once, served as a blob URL. */
export function mockAudioUrl(): string {
  if (audioUrl) return audioUrl;
  const rate = 22050;
  const secs = 8;
  const n = rate * secs;
  const beat = 60 / 82;
  const pcm = new Int16Array(n);
  for (let i = 0; i < n; i++) {
    const t = i / rate;
    const inBeat = t % beat;
    const kick = Math.sin(2 * Math.PI * 52 * inBeat) * Math.exp(-inBeat * 6) * 0.7;
    const hat = (Math.random() * 2 - 1) * Math.exp(-((t % (beat / 2)) * 40)) * 0.08;
    const pad = Math.sin(2 * Math.PI * 174.6 * t) * 0.05 + Math.sin(2 * Math.PI * 207.7 * t) * 0.04;
    pcm[i] = Math.max(-1, Math.min(1, kick + hat + pad)) * 32767;
  }
  const buf = new ArrayBuffer(44 + n * 2);
  const v = new DataView(buf);
  const str = (o: number, s: string) => [...s].forEach((c, i) => v.setUint8(o + i, c.charCodeAt(0)));
  str(0, "RIFF");
  v.setUint32(4, 36 + n * 2, true);
  str(8, "WAVE");
  str(12, "fmt ");
  v.setUint32(16, 16, true);
  v.setUint16(20, 1, true);
  v.setUint16(22, 1, true);
  v.setUint32(24, rate, true);
  v.setUint32(28, rate * 2, true);
  v.setUint16(32, 2, true);
  v.setUint16(34, 16, true);
  str(36, "data");
  v.setUint32(40, n * 2, true);
  new Int16Array(buf, 44).set(pcm);
  audioUrl = URL.createObjectURL(new Blob([buf], { type: "audio/wav" }));
  return audioUrl;
}

// ------------------------------------------------------------------ lyrics

const HOOKS = [
  "you saw it and you let it sit",
  "I know the way home with my eyes closed",
  "say who was there, say it slow",
  "the rooms got bigger and the nights got long",
  "same street, same light, different me",
];

/** Template lyrics in the ACE-Step format, long enough to judge the editor. */
export function mockLyrics(theme: string, mood: string, durationSec: number): string {
  const r = seeded(theme + mood);
  const hook = HOOKS[Math.floor(r() * HOOKS.length)];
  const verse = (n: number) =>
    [
      `[verse]`,
      n === 1 ? `Quarter past two on Queen and Spadina` : `Third floor, no lights, the lease in my name`,
      n === 1 ? `phone face down, still feel it light up` : `you said you'd call, I said the same`,
      `left the car running, left the coat on`,
      `every word I typed I already read back`,
      `rain on the windshield doing what I can't`,
      `I don't chase, that's the story that I tell`,
      `but the way I check the time says it well`,
      `${theme.split(" ").slice(0, 6).join(" ")}, yeah`,
    ].join("\n");
  const chorus = [`[chorus]`, hook, hook, `and I'm still here, still here`, hook].join("\n");
  const bridge = [`[bridge]`, `if you're up, you're up, I know you're up`, `blue light on your face like a streetlamp`, `I'm not asking, I'm just saying`].join("\n");
  const parts = ["[intro]\nmm, yeah\nlate", verse(1), chorus, verse(2), chorus];
  if (durationSec >= 120) parts.push(bridge, chorus);
  if (durationSec >= 210) parts.push(verse(1).replace("[verse]", "[verse]").replace("Quarter past two", "Half past four"), chorus);
  return parts.join("\n\n") + "\n\n[outro]\n" + hook + "\nmm";
}

export function mockTitle(theme: string): string {
  const stop = new Set(["the", "a", "an", "and", "at", "of", "in", "on", "to", "she", "he", "it", "never", "was", "were", "is", "are", "my", "her", "his"]);
  const words = theme
    .replace(/[^a-z0-9 ]/gi, " ")
    .split(/\s+/)
    .filter((w) => w && !stop.has(w.toLowerCase()));
  const pick = words.slice(0, 3).length ? words.slice(0, 3) : ["Untitled"];
  return pick.map((w) => w[0].toUpperCase() + w.slice(1).toLowerCase()).join(" ");
}

// ------------------------------------------------------------------- songs

const doneStages: Stages = { lyrics: "done", render: "done", separate: "done", convert: "skipped", mix: "done" };
const doneFiles = {
  raw: "raw.wav",
  vocals: "vocals.wav",
  instrumental: "instrumental.wav",
  converted: null,
  final: "final.wav",
  mp3: "final.mp3",
  cover: "cover.png",
};

const hoursAgo = (h: number) => new Date(Date.now() - h * 3600_000).toISOString();

export const mockSongs: Song[] = [
  {
    id: "20260929-021412-k3q9",
    title: "Left On Read",
    theme: "she saw the message at 2am and never answered",
    mood: "Heartbreak",
    createdAt: hoursAgo(3),
    persona: defaultPersona,
    lyrics: mockLyrics("she saw the message at 2am and never answered", "Heartbreak", 150),
    tags: defaultPersona.tags + ", heartbreak, 2am, phone screen glow, sparse piano, slow build",
    bpm: 82,
    key: "F minor",
    durationSec: 150,
    engine: "local",
    seed: 1934251,
    steps: 60,
    stages: doneStages,
    error: null,
    files: doneFiles,
    actualDurationSec: 148.3,
    renderSeconds: 214.6,
  },
  {
    id: "20260928-233007-b7xd",
    title: "Gardiner At Three",
    theme: "driving the expressway alone after the party, nothing on the radio",
    mood: "Late night drive",
    createdAt: hoursAgo(27),
    persona: defaultPersona,
    lyrics: mockLyrics("driving the expressway alone after the party", "Late night drive", 210),
    tags: defaultPersona.tags + ", night drive, wet asphalt, rolling bass, wide reverb, headlights",
    bpm: 80,
    key: "C# minor",
    durationSec: 210,
    engine: "fal",
    seed: 7728104,
    steps: 60,
    stages: doneStages,
    error: null,
    files: doneFiles,
    actualDurationSec: 209.1,
    renderSeconds: 61.2,
  },
  {
    id: "20260927-013340-p2mw",
    title: "Who Was There",
    theme: "the friends who were around before the money and the ones who showed up after",
    mood: "Loyalty",
    createdAt: hoursAgo(52),
    persona: defaultPersona,
    lyrics: mockLyrics("the friends who were around before the money", "Loyalty", 90),
    tags: defaultPersona.tags + ", loyalty, warm keys, sparse, confessional, half-time drums",
    bpm: 84,
    key: "A minor",
    durationSec: 90,
    engine: "local",
    seed: 4405881,
    steps: 27,
    stages: { ...doneStages, convert: "done" },
    error: null,
    files: { ...doneFiles, converted: "vocals_converted.wav" },
    actualDurationSec: 91.7,
    renderSeconds: 96.4,
  },
  {
    id: "20260926-034501-t9ne",
    title: "Penthouse Echo",
    theme: "the apartment is bigger now and quieter than it should be",
    mood: "Flex",
    createdAt: hoursAgo(80),
    persona: defaultPersona,
    lyrics: mockLyrics("the apartment is bigger now and quieter than it should be", "Flex", 150),
    tags: defaultPersona.tags + ", flex, glass and marble, distant sirens, plucked synth, empty rooms",
    bpm: 82,
    key: "G minor",
    durationSec: 150,
    engine: "local",
    seed: 1120557,
    steps: 60,
    stages: { lyrics: "done", render: "error", separate: "pending", convert: "skipped", mix: "pending" },
    error: "ACE-Step ran out of memory at step 41/60 (MPS). Try 27 steps or a shorter song.",
    files: { raw: null, vocals: null, instrumental: null, converted: null, final: null, mp3: null, cover: null },
    actualDurationSec: null,
    renderSeconds: null,
  },
];

// ------------------------------------------------------------------- setup

export const mockCheck: EngineCheck = {
  python: "3.10.14",
  torch: "2.5.1",
  device: "mps",
  acestep: true,
  demucs: true,
  rvc: false,
  ffmpeg: "/opt/homebrew/bin/ffmpeg",
  models: { acestep: true, demucs: true, rvcAssets: false },
  problems: ["rvc-python is not installed; voice conversion will be skipped until you run Install engine again."],
};

export const mockSetup: SetupStatus = {
  dataDir: "/Users/you/Library/Application Support/com.overnight.studio",
  claude: "/Users/you/.local/bin/claude",
  uv: "/Users/you/.local/bin/uv",
  ffmpeg: "/opt/homebrew/bin/ffmpeg",
  engineSource: "/Users/you/projects/overnight/engine",
  venv: true,
  check: mockCheck,
  installing: false,
};

/** What setup.sh prints, roughly, for the live install log. */
export const mockInstallLines = [
  "uv 0.4.18 at /Users/you/.local/bin/uv",
  "uv venv --python 3.10 venv",
  "Using CPython 3.10.14",
  "Creating virtual environment at: venv",
  "Installing torch==2.5.1 torchaudio==2.5.1",
  "Resolved 21 packages in 1.2s",
  "Installed 21 packages in 9.8s",
  "Installing ACE-Step from github (pinned)",
  "Installed 48 packages in 22.4s",
  "Installing demucs rvc-python Pillow soundfile numpy",
  "Installed 17 packages in 6.1s",
  "python -m overnight_engine check",
  "device: mps",
  "Done. Model weights download on first render.",
];
