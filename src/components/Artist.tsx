// Artist: the persona Claude writes as and the style tags the music model
// hears. Plain form, explicit save, a preview of the tags as the engine will
// see them.

import { useEffect, useState } from "react";
import type { Persona } from "../types";
import { getPersona, resetPersona, setPersona } from "../api";
import { errText, splitTags } from "../util";
import ConfirmButton from "./ConfirmButton";

interface Props {
  onChange: (p: Persona) => void;
}

const FIELDS: Array<{ key: keyof Persona; label: string; hint: string; rows?: number }> = [
  { key: "name", label: "Name", hint: "Goes on the cover and in the mp3 tags." },
  { key: "tagline", label: "Tagline", hint: "One line under the name." },
  { key: "voice", label: "Voice", hint: "How the vocal should sound. Claude uses it to pick words that sit in that delivery.", rows: 2 },
  { key: "tags", label: "Style tags", hint: "Comma separated. Every song starts from these and adds a few of its own. Genre, mood, instruments, vocal, bpm, key. Never an artist's name.", rows: 3 },
  { key: "themes", label: "Themes", hint: "What the songs tend to be about.", rows: 2 },
  { key: "voiceNotes", label: "Writing notes", hint: "Point of view, density, the kind of detail, how hooks work.", rows: 3 },
  { key: "rules", label: "Rules", hint: "Hard lines Claude never crosses.", rows: 2 },
  { key: "coverStyle", label: "Cover style", hint: "A few colour words steer the cover art.", rows: 2 },
];

export default function Artist({ onChange }: Props) {
  const [saved, setSaved] = useState<Persona | null>(null);
  const [draft, setDraft] = useState<Persona | null>(null);
  const [busy, setBusy] = useState(false);
  const [note, setNote] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    getPersona()
      .then((p) => {
        setSaved(p);
        setDraft(p);
      })
      .catch((e) => setError(errText(e)));
  }, []);

  if (!draft || !saved) return <section className="artist">{error ? <p className="error">{error}</p> : <p className="dim">Loading</p>}</section>;

  const dirty = JSON.stringify(draft) !== JSON.stringify(saved);
  const set = (key: keyof Persona, value: string) => {
    setDraft({ ...draft, [key]: value });
    setNote(null);
  };

  const run = async (job: () => Promise<Persona>, done: string) => {
    setBusy(true);
    setError(null);
    try {
      const p = await job();
      setSaved(p);
      setDraft(p);
      onChange(p);
      setNote(done);
    } catch (e) {
      setError(errText(e));
    } finally {
      setBusy(false);
    }
  };

  const tags = splitTags(draft.tags);

  return (
    <section className="artist">
      <header className="page-head">
        <h1 className="display">{draft.name || "Artist"}</h1>
        <span className="dim">{draft.tagline}</span>
      </header>
      <div className="artist-grid">
        <form
          className="form"
          onSubmit={(e) => {
            e.preventDefault();
            void run(() => setPersona(draft), "Saved.");
          }}
        >
          {FIELDS.map((f) => (
            <label key={f.key} className="field">
              <span className="label">{f.label}</span>
              {f.rows ? (
                <textarea className="input" rows={f.rows} value={draft[f.key]} onChange={(e) => set(f.key, e.target.value)} />
              ) : (
                <input className="input" value={draft[f.key]} onChange={(e) => set(f.key, e.target.value)} />
              )}
              <span className="hint">{f.hint}</span>
            </label>
          ))}
          <div className="btn-row">
            <button type="submit" className="btn primary" disabled={!dirty || busy}>
              Save
            </button>
            <button type="button" className="btn quiet" disabled={!dirty || busy} onClick={() => setDraft(saved)}>
              Discard changes
            </button>
            <ConfirmButton className="btn quiet" label="Reset to default" confirmLabel="Reset? This replaces every field" disabled={busy} onConfirm={() => void run(resetPersona, "Back to the default persona.")} />
            {note ? <span className="hint">{note}</span> : null}
          </div>
          {error ? (
            <p className="error" role="alert">
              {error}
            </p>
          ) : null}
        </form>
        <aside className="tags-preview" aria-live="polite">
          <span className="label">
            Tags as the model hears them <span className="num dim">({tags.length})</span>
          </span>
          <div className="chips">
            {tags.map((t) => (
              <span key={t} className="chip static">
                {t}
              </span>
            ))}
          </div>
          <pre className="tags-raw">{tags.join(", ")}</pre>
          <p className="hint">Each song appends three to six descriptors of its own after these.</p>
        </aside>
      </div>
    </section>
  );
}
