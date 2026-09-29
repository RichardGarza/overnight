// The Studio's state lives above the page so switching to Library or Setup
// while a song renders does not lose the render (the render_song call keeps
// running and its result lands here when it comes back).

import { useCallback, useEffect, useRef, useState } from "react";
import type { ProgressEvent, RenderFrom, Song, SongPatch } from "./types";
import { cancelRender, deleteSong, getSong, onProgress, renderSong, updateSong, writeSong } from "./api";
import { errText } from "./util";

export type Phase = "compose" | "review" | "rendering" | "done";

export interface StudioSession {
  song: Song | null;
  phase: Phase;
  /** Progress events for the current song, in order. */
  events: ProgressEvent[];
  /** Latest event per song id, for the Library's mid-render badges. */
  live: Record<string, ProgressEvent>;
  writing: boolean;
  /** How long Claude took on the lyrics, for the stage list. */
  writeMs: number | null;
  error: string | null;
  /** Bumped whenever a song was created, changed or removed; the Library reloads on it. */
  version: number;
  write: (theme: string, mood: string, durationSec: number) => Promise<void>;
  rewrite: () => Promise<void>;
  open: (song: Song) => void;
  patch: (p: SongPatch) => Promise<void>;
  render: (from: RenderFrom) => Promise<void>;
  cancel: () => Promise<void>;
  reset: () => void;
  showReview: () => void;
  bump: () => void;
}

const isFinished = (s: Song) => s.stages.mix === "done" && !!s.files.final;

export function useStudio(): StudioSession {
  const [song, setSong] = useState<Song | null>(null);
  const [phase, setPhase] = useState<Phase>("compose");
  const [events, setEvents] = useState<ProgressEvent[]>([]);
  const [live, setLive] = useState<Record<string, ProgressEvent>>({});
  const [writing, setWriting] = useState(false);
  const [writeMs, setWriteMs] = useState<number | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [version, setVersion] = useState(0);
  const songRef = useRef<Song | null>(null);
  songRef.current = song;
  const bump = useCallback(() => setVersion((v) => v + 1), []);

  useEffect(() => {
    let off: (() => void) | undefined;
    onProgress((e) => {
      setLive((prev) => ({ ...prev, [e.songId]: e }));
      if (e.songId === songRef.current?.id) setEvents((prev) => [...prev.slice(-400), e]);
    }).then((fn) => (off = fn));
    return () => off?.();
  }, []);

  const write = useCallback(
    async (theme: string, mood: string, durationSec: number) => {
      setWriting(true);
      setError(null);
      const t0 = Date.now();
      try {
        const s = await writeSong(theme, mood, durationSec);
        setWriteMs(Date.now() - t0);
        setSong(s);
        setEvents([]);
        setPhase("review");
        bump();
      } catch (e) {
        setError(errText(e));
      } finally {
        setWriting(false);
      }
    },
    [bump]
  );

  // Rewrite asks for fresh lyrics on the same brief. The previous draft never
  // made a sound, so it is removed rather than left to clutter the Library.
  const rewrite = useCallback(async () => {
    const prev = songRef.current;
    if (!prev) return;
    await write(prev.theme, prev.mood, prev.durationSec);
    if (!prev.files.raw) deleteSong(prev.id).then(bump).catch(() => {});
  }, [write, bump]);

  const open = useCallback((s: Song) => {
    setSong(s);
    setEvents([]);
    setError(s.error);
    setWriteMs(null);
    const running = Object.values(s.stages).includes("running");
    setPhase(running ? "rendering" : isFinished(s) ? "done" : "review");
  }, []);

  const patch = useCallback(
    async (p: SongPatch) => {
      const cur = songRef.current;
      if (!cur) return;
      const changed: SongPatch = {};
      if (p.title !== undefined && p.title !== cur.title) changed.title = p.title;
      if (p.tags !== undefined && p.tags !== cur.tags) changed.tags = p.tags;
      if (p.lyrics !== undefined && p.lyrics !== cur.lyrics) changed.lyrics = p.lyrics;
      if (p.durationSec !== undefined && p.durationSec !== cur.durationSec) changed.durationSec = p.durationSec;
      if (Object.keys(changed).length === 0) return;
      const saved = await updateSong(cur.id, changed);
      setSong(saved);
      bump();
    },
    [bump]
  );

  const render = useCallback(
    async (from: RenderFrom) => {
      const cur = songRef.current;
      if (!cur) return;
      setError(null);
      setEvents([]);
      setPhase("rendering");
      // Mark the stages we are about to run as pending so stale done/error
      // marks from a previous run do not show while the engine warms up.
      const order: RenderFrom[] = ["render", "separate", "convert", "mix"];
      const stages = { ...cur.stages };
      for (const st of order.slice(order.indexOf(from))) if (stages[st] !== "skipped") stages[st] = "pending";
      setSong({ ...cur, stages, error: null });
      try {
        const done = await renderSong(cur.id, from);
        setSong(done);
        setPhase(isFinished(done) ? "done" : "review");
        if (done.error) setError(done.error);
      } catch (e) {
        setError(errText(e));
        // The backend has the truth about which stage failed; read it back.
        try {
          const fresh = await getSong(cur.id);
          setSong(fresh);
        } catch {
          /* keep what we had */
        }
        setPhase("rendering");
      } finally {
        bump();
      }
    },
    [bump]
  );

  const cancel = useCallback(async () => {
    const cur = songRef.current;
    if (!cur) return;
    try {
      await cancelRender(cur.id);
    } catch (e) {
      setError(errText(e));
    }
  }, []);

  const reset = useCallback(() => {
    setSong(null);
    setEvents([]);
    setError(null);
    setWriteMs(null);
    setPhase("compose");
  }, []);

  const showReview = useCallback(() => setPhase("review"), []);

  return { song, phase, events, live, writing, writeMs, error, version, write, rewrite, open, patch, render, cancel, reset, showReview, bump };
}
