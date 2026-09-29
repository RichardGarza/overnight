// Library: every song as a cover in a grid. Finished songs play in the bar
// along the bottom; unfinished ones open in the Studio where they left off.

import { useEffect, useState } from "react";
import type { ProgressEvent, Song, StageName } from "../types";
import { deleteSong, listSongs, songFileUrl } from "../api";
import { ago, errText, mmss } from "../util";
import ConfirmButton from "./ConfirmButton";
import Player from "./Player";

interface Props {
  dataDir: string;
  /** Latest progress event per song id, from the global listener. */
  live: Record<string, ProgressEvent>;
  /** Changes whenever a song was written, rendered or deleted elsewhere. */
  version: number;
  onOpen: (song: Song) => void;
}

const STAGE_WORD: Record<StageName, string> = { lyrics: "Writing", render: "Rendering", separate: "Separating", convert: "Converting", mix: "Mixing" };

function stageOf(song: Song, ev: ProgressEvent | undefined): { text: string; pct: number | null; failed: boolean } | null {
  const failed = (Object.keys(song.stages) as StageName[]).find((k) => song.stages[k] === "error");
  if (failed && (!ev || ev.event === "error" || ev.event === "done")) return { text: `Failed at ${failed}`, pct: null, failed: true };
  const running = (Object.keys(song.stages) as StageName[]).find((k) => song.stages[k] === "running");
  if (ev && ev.event !== "done" && ev.event !== "error" && ev.stage !== "setup") {
    return { text: STAGE_WORD[ev.stage], pct: ev.stage === "render" ? (ev.pct ?? 0) : null, failed: false };
  }
  if (running) return { text: STAGE_WORD[running], pct: null, failed: false };
  if (!song.files.final) return { text: song.files.raw ? "Not mixed" : "Lyrics only", pct: null, failed: false };
  return null;
}

export default function Library({ dataDir, live, version, onOpen }: Props) {
  const [songs, setSongs] = useState<Song[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [playing, setPlaying] = useState<Song | null>(null);

  useEffect(() => {
    let alive = true;
    listSongs()
      .then((s) => {
        if (!alive) return;
        setSongs(s);
        // keep the bottom bar's song fresh (a re-mix changes its files)
        setPlaying((p) => (p ? (s.find((x) => x.id === p.id) ?? null) : null));
      })
      .catch((e) => alive && setError(errText(e)));
    return () => {
      alive = false;
    };
  }, [version]);

  const remove = async (song: Song) => {
    try {
      await deleteSong(song.id);
      setSongs((prev) => prev?.filter((s) => s.id !== song.id) ?? null);
      if (playing?.id === song.id) setPlaying(null);
    } catch (e) {
      setError(errText(e));
    }
  };

  const open = (song: Song) => {
    if (song.files.final && song.stages.mix === "done") setPlaying(song);
    else onOpen(song);
  };

  return (
    <section className={`library ${playing ? "with-bar" : ""}`}>
      <header className="page-head">
        <h1 className="display">Library</h1>
        {songs ? <span className="dim num">{songs.length === 1 ? "1 song" : `${songs.length} songs`}</span> : null}
      </header>
      {error ? (
        <p className="error" role="alert">
          {error}
        </p>
      ) : null}
      {songs && songs.length === 0 ? (
        <div className="empty">
          <p className="display">Nothing here yet.</p>
          <p className="dim">Write one in the Studio. It lands here when it's mixed.</p>
        </div>
      ) : null}
      <ul className="grid" aria-label="Songs">
        {(songs ?? []).map((song) => {
          const st = stageOf(song, live[song.id]);
          const cover = song.files.cover ? songFileUrl(dataDir, song, song.files.cover) : null;
          const isPlaying = playing?.id === song.id;
          return (
            <li key={song.id} className={`card ${isPlaying ? "playing" : ""} ${st?.failed ? "failed" : ""}`}>
              <button type="button" className="card-hit" onClick={() => open(song)} aria-label={`${song.title}${st ? `, ${st.text}` : ", play"}`}>
                <div className="cover">
                  {cover ? (
                    <img src={cover} alt="" />
                  ) : (
                    <div className="cover-blank">
                      <span>{song.title}</span>
                    </div>
                  )}
                  {st ? (
                    <div className={`cover-state ${st.failed ? "err" : ""}`}>
                      {!st.failed && st.text !== "Lyrics only" && st.text !== "Not mixed" ? <span className="spin" aria-hidden="true" /> : null}
                      <span>{st.text}</span>
                      {st.pct != null ? <em className="num">{Math.round(st.pct)}%</em> : null}
                      {st.pct != null ? (
                        <div className="bar thin">
                          <span style={{ width: `${st.pct}%` }} />
                        </div>
                      ) : null}
                    </div>
                  ) : null}
                </div>
                <div className="card-title">{song.title}</div>
                <div className="card-meta num">
                  <span>{ago(song.createdAt)}</span>
                  <span>{mmss(song.actualDurationSec ?? song.durationSec)}</span>
                  <span>{song.engine}</span>
                </div>
              </button>
              <ConfirmButton
                className="icon-btn card-delete"
                ariaLabel={`Delete ${song.title}`}
                title="Delete"
                label={
                  <svg viewBox="0 0 16 16" width="14" height="14" aria-hidden="true">
                    <path d="M3 4h10M6 4V2.5h4V4M4.5 4l.7 9.5h5.6l.7-9.5" stroke="currentColor" strokeWidth="1.4" fill="none" />
                  </svg>
                }
                confirmLabel="Delete?"
                onConfirm={() => void remove(song)}
              />
            </li>
          );
        })}
      </ul>
      {playing ? (
        <div className="player-bar">
          <Player song={playing} dataDir={dataDir} compact autoPlay onClose={() => setPlaying(null)} />
        </div>
      ) : null}
    </section>
  );
}
