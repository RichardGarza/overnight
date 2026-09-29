// The five stages as a vertical list. Each row shows a state icon, the last
// message the engine sent for that stage, how long it has taken, and for the
// render stage a percentage bar. State comes from the song record and is
// overlaid with the live events so the list moves as the engine works.

import { useEffect, useState } from "react";
import type { ProgressEvent, StageName, StageState, Song } from "../types";
import { elapsed } from "../util";

interface Props {
  song: Song;
  events: ProgressEvent[];
  writeMs: number | null;
  rendering: boolean;
}

const ORDER: Array<{ key: StageName; label: string; hint: string }> = [
  { key: "lyrics", label: "Lyrics", hint: "Claude writes the words" },
  { key: "render", label: "Render", hint: "ACE-Step sings and plays them" },
  { key: "separate", label: "Separate", hint: "demucs splits vocal and beat" },
  { key: "convert", label: "Convert", hint: "RVC re-sings the vocal" },
  { key: "mix", label: "Mix", hint: "ffmpeg mixes, masters and tags" },
];

interface Row {
  state: StageState;
  message: string;
  detail?: string;
  pct?: number;
  startedAt?: number;
  endedAt?: number;
}

export function deriveRows(song: Song, events: ProgressEvent[], writeMs: number | null): Record<StageName, Row> {
  const rows = {} as Record<StageName, Row>;
  for (const { key } of ORDER) rows[key] = { state: song.stages[key], message: "" };
  if (song.stages.lyrics === "done") {
    const lines = song.lyrics.split("\n").filter((l) => l.trim() && !l.trim().startsWith("[")).length;
    rows.lyrics.message = `${lines} lines, ${song.bpm} bpm, ${song.key}`;
    if (writeMs != null) rows.lyrics.detail = elapsed(writeMs);
  }
  for (const e of events) {
    if (e.stage === "setup") continue;
    const r = rows[e.stage];
    if (e.event === "start") {
      r.state = "running";
      r.startedAt = e.at;
      r.endedAt = undefined;
      r.pct = 0;
      r.message = e.message;
    } else if (e.event === "progress") {
      if (r.state === "pending") r.state = "running";
      r.startedAt ??= e.at;
      if (e.pct != null) r.pct = e.pct;
      r.message = e.message;
    } else if (e.event === "log") {
      if (r.state === "running") r.message = e.message;
    } else if (e.event === "done") {
      r.state = "done";
      r.endedAt = e.at;
      r.pct = 100;
      r.message = e.message;
      r.detail = e.detail;
    } else if (e.event === "error") {
      r.state = "error";
      r.endedAt = e.at;
      r.message = e.message;
      r.detail = e.detail;
    }
  }
  if (song.error && !events.some((e) => e.event === "error")) {
    const failed = ORDER.find((s) => song.stages[s.key] === "error");
    if (failed) rows[failed.key].message = song.error;
  }
  return rows;
}

function Icon({ state }: { state: StageState }) {
  switch (state) {
    case "done":
      return (
        <svg viewBox="0 0 16 16" width="16" height="16" aria-hidden="true">
          <path d="M3 8.5l3.2 3L13 4.5" stroke="currentColor" strokeWidth="1.8" fill="none" />
        </svg>
      );
    case "running":
      return <span className="spin" aria-hidden="true" />;
    case "error":
      return (
        <svg viewBox="0 0 16 16" width="16" height="16" aria-hidden="true">
          <path d="M4 4l8 8M12 4l-8 8" stroke="currentColor" strokeWidth="1.8" fill="none" />
        </svg>
      );
    case "skipped":
      return (
        <svg viewBox="0 0 16 16" width="16" height="16" aria-hidden="true">
          <path d="M3 8h10" stroke="currentColor" strokeWidth="1.8" fill="none" />
        </svg>
      );
    default:
      return <span className="dot" aria-hidden="true" />;
  }
}

const WORD: Record<StageState, string> = { pending: "waiting", running: "running", done: "done", error: "failed", skipped: "skipped" };

export default function StageList({ song, events, writeMs, rendering }: Props) {
  const [, tick] = useState(0);
  useEffect(() => {
    if (!rendering) return;
    const t = setInterval(() => tick((n) => n + 1), 500);
    return () => clearInterval(t);
  }, [rendering]);

  const rows = deriveRows(song, events, writeMs);
  const now = Date.now();

  return (
    <ol className="stages" aria-label="Stages">
      {ORDER.map(({ key, label, hint }) => {
        const r = rows[key];
        const ms = r.startedAt ? (r.endedAt ?? now) - r.startedAt : null;
        const time = r.detail && r.state !== "running" ? r.detail : ms != null ? elapsed(ms) : "";
        return (
          <li key={key} className={`stage ${r.state}`}>
            <span className="stage-icon" title={WORD[r.state]}>
              <Icon state={r.state} />
              <span className="sr-only">{WORD[r.state]}</span>
            </span>
            <div className="stage-main">
              <div className="stage-head">
                <span className="stage-name">{label}</span>
                <span className="stage-time num">{time}</span>
              </div>
              <div className={`stage-msg ${r.state === "error" ? "err" : ""}`}>{r.message || hint}</div>
              {key === "render" && (r.state === "running" || (r.state === "done" && r.pct != null)) ? (
                <div className="bar-row">
                  <div className="bar" role="progressbar" aria-label="Render progress" aria-valuemin={0} aria-valuemax={100} aria-valuenow={r.pct ?? 0}>
                    <span style={{ width: `${r.pct ?? 0}%` }} />
                  </div>
                  <em className="num">{Math.round(r.pct ?? 0)}%</em>
                </div>
              ) : null}
            </div>
          </li>
        );
      })}
    </ol>
  );
}
