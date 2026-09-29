// The player: cover, title, persona, a waveform-looking progress bar that
// seeks on click, and the time in tabular figures. The "waveform" is
// deterministic per song id, not an analysis of the audio; it exists so the
// eye has something to follow while the song plays.

import { useEffect, useMemo, useRef, useState } from "react";
import type { Song } from "../types";
import { songFileUrl } from "../api";
import { mmss, seeded } from "../util";

interface Props {
  song: Song;
  dataDir: string;
  /** The Library's bottom bar: one row, smaller cover. */
  compact?: boolean;
  autoPlay?: boolean;
  onClose?: () => void;
}

const BARS = 72;

export default function Player({ song, dataDir, compact, autoPlay, onClose }: Props) {
  const audio = useRef<HTMLAudioElement>(null);
  const [playing, setPlaying] = useState(false);
  const [time, setTime] = useState(0);
  const [duration, setDuration] = useState(song.actualDurationSec ?? song.durationSec);
  const [failed, setFailed] = useState(false);

  const file = song.files.mp3 ?? song.files.final;
  const src = file ? songFileUrl(dataDir, song, file) : null;
  const cover = song.files.cover ? songFileUrl(dataDir, song, song.files.cover) : null;

  const bars = useMemo(() => {
    const r = seeded(song.id);
    return Array.from({ length: BARS }, (_, i) => {
      // quieter at the very start and end, like a real song
      const edge = Math.min(1, Math.min(i, BARS - 1 - i) / 6 + 0.25);
      return Math.round((0.25 + r() * 0.75) * edge * 100);
    });
  }, [song.id]);

  useEffect(() => {
    setPlaying(false);
    setTime(0);
    setFailed(false);
    setDuration(song.actualDurationSec ?? song.durationSec);
    const a = audio.current;
    if (a && autoPlay && src) a.play().catch(() => {});
  }, [song.id, src, autoPlay]);

  const toggle = () => {
    const a = audio.current;
    if (!a) return;
    if (a.paused) a.play().catch(() => setFailed(true));
    else a.pause();
  };

  const seek = (e: React.MouseEvent<HTMLDivElement>) => {
    const a = audio.current;
    if (!a || !duration) return;
    const r = e.currentTarget.getBoundingClientRect();
    a.currentTime = Math.max(0, Math.min(1, (e.clientX - r.left) / r.width)) * duration;
  };

  const pct = duration ? Math.min(100, (time / duration) * 100) : 0;

  return (
    <div className={`player ${compact ? "compact" : ""}`}>
      {cover ? <img className="player-cover" src={cover} alt="" /> : <div className="player-cover blank" aria-hidden="true" />}
      <div className="player-body">
        <div className="player-head">
          <div className="player-titles">
            <div className="player-title">{song.title}</div>
            <div className="player-sub">
              {song.persona.name}
              {song.key ? <span className="dim"> · {song.key}</span> : null}
              {song.bpm ? <span className="dim"> · {song.bpm} bpm</span> : null}
            </div>
          </div>
          {onClose ? (
            <button type="button" className="icon-btn" aria-label="Close player" onClick={onClose}>
              <svg viewBox="0 0 16 16" width="14" height="14" aria-hidden="true">
                <path d="M3 3l10 10M13 3L3 13" stroke="currentColor" strokeWidth="1.8" fill="none" />
              </svg>
            </button>
          ) : null}
        </div>
        <div className="player-row">
          <button type="button" className="play-btn" aria-label={playing ? "Pause" : "Play"} onClick={toggle} disabled={!src}>
            {playing ? (
              <svg viewBox="0 0 16 16" width="16" height="16" aria-hidden="true">
                <rect x="3" y="2" width="4" height="12" fill="currentColor" />
                <rect x="9" y="2" width="4" height="12" fill="currentColor" />
              </svg>
            ) : (
              <svg viewBox="0 0 16 16" width="16" height="16" aria-hidden="true">
                <path d="M4 2l10 6-10 6z" fill="currentColor" />
              </svg>
            )}
          </button>
          <div
            className="wave"
            role="slider"
            aria-label="Position"
            aria-valuemin={0}
            aria-valuemax={Math.round(duration)}
            aria-valuenow={Math.round(time)}
            tabIndex={0}
            onClick={seek}
            onKeyDown={(e) => {
              const a = audio.current;
              if (!a) return;
              if (e.key === "ArrowRight") a.currentTime = Math.min(duration, a.currentTime + 5);
              if (e.key === "ArrowLeft") a.currentTime = Math.max(0, a.currentTime - 5);
              if (e.key === " ") {
                e.preventDefault();
                toggle();
              }
            }}
          >
            {bars.map((h, i) => (
              <span key={i} className={i / BARS < pct / 100 ? "on" : ""} style={{ height: `${h}%` }} />
            ))}
          </div>
          <div className="player-time num">
            {mmss(time)} <span className="dim">/ {mmss(duration)}</span>
          </div>
        </div>
        {failed ? <div className="player-note">Could not play {file}.</div> : null}
        {!src ? <div className="player-note">No mix yet.</div> : null}
      </div>
      {src ? (
        <audio
          ref={audio}
          src={src}
          preload="metadata"
          onPlay={() => setPlaying(true)}
          onPause={() => setPlaying(false)}
          onEnded={() => setPlaying(false)}
          onTimeUpdate={(e) => setTime(e.currentTarget.currentTime)}
          onLoadedMetadata={(e) => {
            const d = e.currentTarget.duration;
            if (isFinite(d) && d > 0) setDuration(d);
          }}
          onError={() => setFailed(true)}
        />
      ) : null}
    </div>
  );
}
