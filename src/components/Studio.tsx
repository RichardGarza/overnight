// Studio: brief -> lyrics review -> render -> player. One song at a time.
// The flow state itself lives in useStudio (studio.ts) so it survives a trip
// to the other tabs mid-render.

import { useEffect, useRef, useState } from "react";
import type { RenderFrom, Settings, Song } from "../types";
import type { StudioSession } from "../studio";
import { revealSong } from "../api";
import { mmss } from "../util";
import StageList from "./StageList";
import Player from "./Player";

interface Props {
  session: StudioSession;
  settings: Settings | null;
  dataDir: string;
}

const MOODS = ["Late night drive", "Heartbreak", "Flex", "The city", "Loyalty", "Nostalgia", "Custom"];
const LENGTHS = [90, 150, 210];

export default function Studio({ session, settings, dataDir }: Props) {
  const { song, phase } = session;
  if (phase === "compose" || !song) return <Compose session={session} settings={settings} />;
  if (phase === "review") return <Review session={session} song={song} />;
  if (phase === "rendering") return <Rendering session={session} song={song} />;
  return <Done session={session} song={song} settings={settings} dataDir={dataDir} />;
}

// ------------------------------------------------------------------ compose

function Compose({ session, settings }: { session: StudioSession; settings: Settings | null }) {
  const [theme, setTheme] = useState("");
  const [mood, setMood] = useState("Late night drive");
  const [custom, setCustom] = useState("");
  const [length, setLength] = useState<number | null>(null);
  const box = useRef<HTMLTextAreaElement>(null);

  // The default length comes from Settings; snap it to the nearest chip.
  const fallback = settings?.defaultDurationSec ?? 150;
  const chosen = length ?? LENGTHS.reduce((a, b) => (Math.abs(b - fallback) < Math.abs(a - fallback) ? b : a));

  useEffect(() => {
    box.current?.focus();
  }, []);

  const moodValue = mood === "Custom" ? custom.trim() : mood;
  const ready = theme.trim().length > 0 && !session.writing;
  const submit = () => {
    if (ready) void session.write(theme.trim(), moodValue, chosen);
  };

  return (
    <section className="compose">
      <h1 className="display">What's this one about?</h1>
      <textarea
        ref={box}
        className="theme"
        aria-label="Theme"
        placeholder="she saw the message at 2am and never answered"
        rows={3}
        value={theme}
        disabled={session.writing}
        onChange={(e) => setTheme(e.target.value)}
        onKeyDown={(e) => {
          if ((e.metaKey || e.ctrlKey) && e.key === "Enter") {
            e.preventDefault();
            submit();
          }
        }}
      />
      <div className="field-row">
        <span className="label">Mood</span>
        <div className="chips" role="group" aria-label="Mood">
          {MOODS.map((m) => (
            <button key={m} type="button" className={`chip ${mood === m ? "on" : ""}`} aria-pressed={mood === m} onClick={() => setMood(m)}>
              {m}
            </button>
          ))}
        </div>
        {mood === "Custom" ? (
          <input className="input custom-mood" aria-label="Custom mood" placeholder="your own mood, a few words" value={custom} onChange={(e) => setCustom(e.target.value)} autoFocus />
        ) : null}
      </div>
      <div className="field-row">
        <span className="label">Length</span>
        <div className="chips" role="group" aria-label="Length">
          {LENGTHS.map((l) => (
            <button key={l} type="button" className={`chip num ${chosen === l ? "on" : ""}`} aria-pressed={chosen === l} onClick={() => setLength(l)}>
              {mmss(l)}
            </button>
          ))}
        </div>
      </div>
      <div className="compose-actions">
        <button type="button" className={`btn primary big ${session.writing ? "busy" : ""}`} disabled={!ready} onClick={submit}>
          {session.writing ? "Writing" : "Write it"}
        </button>
        <span className="hint">{session.writing ? "Claude is writing. This takes a minute." : "Cmd + Enter"}</span>
      </div>
      {session.error ? (
        <p className="error" role="alert">
          {session.error}
        </p>
      ) : null}
    </section>
  );
}

// ------------------------------------------------------------------- review

function Review({ session, song }: { session: StudioSession; song: Song }) {
  const [title, setTitle] = useState(song.title);
  const [tags, setTags] = useState(song.tags);
  const [lyrics, setLyrics] = useState(song.lyrics);
  const [busy, setBusy] = useState(false);
  const [problem, setProblem] = useState<string | null>(null);

  // A fresh song (or a rewrite) replaces whatever was being edited.
  useEffect(() => {
    setTitle(song.title);
    setTags(song.tags);
    setLyrics(song.lyrics);
  }, [song.id]);

  const dirty = title !== song.title || tags !== song.tags || lyrics !== song.lyrics;
  const save = () => session.patch({ title: title.trim() || song.title, tags: tags.trim(), lyrics });

  const go = async (from: RenderFrom) => {
    setBusy(true);
    setProblem(null);
    try {
      await save();
      await session.render(from);
    } catch (e) {
      setProblem(String(e));
    } finally {
      setBusy(false);
    }
  };

  const rendered = !!song.files.raw;
  const failedAt = (["render", "separate", "convert", "mix"] as RenderFrom[]).find((s) => song.stages[s] === "error");

  return (
    <section className="review">
      <div className="review-main">
        <input className="title-input display" aria-label="Title" value={title} onChange={(e) => setTitle(e.target.value)} onBlur={() => void save().catch(() => {})} />
        <label className="field">
          <span className="label">Tags</span>
          <textarea className="input tags" rows={2} value={tags} onChange={(e) => setTags(e.target.value)} onBlur={() => void save().catch(() => {})} />
        </label>
        <div className="field">
          <span className="label">Lyrics</span>
          <LyricsEditor value={lyrics} onChange={setLyrics} onBlur={() => void save().catch(() => {})} />
        </div>
      </div>
      <aside className="review-side">
        <dl className="facts">
          <div>
            <dt>BPM</dt>
            <dd className="num">{song.bpm}</dd>
          </div>
          <div>
            <dt>Key</dt>
            <dd>{song.key}</dd>
          </div>
          <div>
            <dt>Length</dt>
            <dd className="num">{mmss(song.durationSec)}</dd>
          </div>
          <div>
            <dt>Engine</dt>
            <dd>{song.engine}</dd>
          </div>
        </dl>
        <div className="brief">
          <span className="label">Brief</span>
          <p>{song.theme}</p>
          {song.mood ? <span className="chip static">{song.mood}</span> : null}
        </div>
        {song.error || problem ? (
          <p className="error" role="alert">
            {problem ?? song.error}
          </p>
        ) : null}
        <div className="review-actions">
          <button type="button" className="btn primary big" disabled={busy || session.writing} onClick={() => void go("render")}>
            {rendered ? "Render again" : "Render"}
          </button>
          {failedAt && failedAt !== "render" ? (
            <button type="button" className="btn" disabled={busy} onClick={() => void go(failedAt)}>
              Retry from {failedAt}
            </button>
          ) : null}
          <button type="button" className={`btn ${session.writing ? "busy" : ""}`} disabled={busy || session.writing} onClick={() => void session.rewrite()}>
            {session.writing ? "Rewriting" : "Rewrite"}
          </button>
          <button type="button" className="btn quiet" disabled={busy || session.writing} onClick={() => session.reset()}>
            New song
          </button>
        </div>
        <p className="hint">{dirty ? "Edits save when you leave the field or press Render." : "Saved."}</p>
      </aside>
    </section>
  );
}

/**
 * A textarea with the section tags ([verse], [chorus], ...) picked out in
 * amber. The textarea's own text is transparent; a matching <pre> underneath
 * draws the coloured copy and scrolls in step with it.
 */
function LyricsEditor({ value, onChange, onBlur }: { value: string; onChange: (v: string) => void; onBlur: () => void }) {
  const back = useRef<HTMLPreElement>(null);
  const parts = value.split(/(\[[^\]\n]*\])/g);
  return (
    <div className="lyrics-wrap">
      <pre className="lyrics-backdrop" ref={back} aria-hidden="true">
        {parts.map((p, i) => (/^\[[^\]\n]*\]$/.test(p) ? <mark key={i}>{p}</mark> : p))}
        {"\n"}
      </pre>
      <textarea
        className="lyrics"
        aria-label="Lyrics"
        spellCheck={false}
        value={value}
        onChange={(e) => onChange(e.target.value)}
        onBlur={onBlur}
        onScroll={(e) => {
          if (back.current) back.current.scrollTop = e.currentTarget.scrollTop;
        }}
      />
    </div>
  );
}

// ---------------------------------------------------------------- rendering

function Rendering({ session, song }: { session: StudioSession; song: Song }) {
  const failedAt = (["render", "separate", "convert", "mix"] as RenderFrom[]).find((s) => song.stages[s] === "error");
  const stopped = !!session.error || !!failedAt;
  const [cancelling, setCancelling] = useState(false);
  useEffect(() => {
    if (stopped) setCancelling(false);
  }, [stopped]);

  return (
    <section className="rendering">
      <header className="render-head">
        <h1 className="display">{song.title}</h1>
        <p className="dim">{song.theme}</p>
      </header>
      <StageList song={song} events={session.events} writeMs={session.writeMs} rendering={!stopped} />
      {stopped ? (
        <div className="render-actions">
          {session.error && !session.events.some((e) => e.event === "error" && e.message === session.error) ? (
            <p className="error" role="alert">
              {session.error}
            </p>
          ) : null}
          <div className="btn-row">
            <button type="button" className="btn primary" onClick={() => void session.render(failedAt ?? "render")}>
              Try again{failedAt ? ` from ${failedAt}` : ""}
            </button>
            <button type="button" className="btn" onClick={() => session.showReview()}>
              Back to lyrics
            </button>
            <button type="button" className="btn quiet" onClick={() => session.reset()}>
              New song
            </button>
          </div>
        </div>
      ) : (
        <div className="render-actions">
          <button
            type="button"
            className="btn"
            disabled={cancelling}
            onClick={() => {
              setCancelling(true);
              void session.cancel();
            }}
          >
            {cancelling ? "Cancelling" : "Cancel"}
          </button>
        </div>
      )}
    </section>
  );
}

// --------------------------------------------------------------------- done

function Done({ session, song, settings, dataDir }: { session: StudioSession; song: Song; settings: Settings | null; dataDir: string }) {
  const canSing = !!settings?.convert && !!settings.voiceModel.pth;
  return (
    <section className="done">
      <Player song={song} dataDir={dataDir} />
      <div className="done-facts num">
        {song.actualDurationSec != null ? <span>{mmss(song.actualDurationSec)}</span> : null}
        {song.renderSeconds != null ? <span>rendered in {mmss(song.renderSeconds)}</span> : null}
        <span>{song.engine}</span>
        <span>seed {song.seed}</span>
      </div>
      <div className="btn-row">
        <button type="button" className="btn" onClick={() => void revealSong(song.id)}>
          Reveal in Finder
        </button>
        <button type="button" className="btn" onClick={() => void session.render("mix")} title="Mix and master again from the stems">
          Re-mix
        </button>
        <button
          type="button"
          className="btn"
          disabled={!canSing}
          title={canSing ? "Run voice conversion on the vocal again, then mix" : "Turn on voice conversion and pick a model in Setup"}
          onClick={() => void session.render("convert")}
        >
          Re-sing
        </button>
        <button type="button" className="btn quiet" onClick={() => session.showReview()}>
          Lyrics
        </button>
        <button type="button" className="btn primary" onClick={() => session.reset()}>
          New song
        </button>
      </div>
      {session.error ? (
        <p className="error" role="alert">
          {session.error}
        </p>
      ) : null}
    </section>
  );
}
