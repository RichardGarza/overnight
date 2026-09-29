// Setup: what is installed (a checklist), the engine installer with its live
// log, and the settings the engine runs with. Settings save explicitly.

import { useEffect, useRef, useState } from "react";
import type { LogEvent, Settings, SetupStatus, VoiceModel } from "../types";
import { getSettings, installEngine, onLog, openDataDir, pickFile, setSettings, setupStatus } from "../api";
import { errText } from "../util";

interface Props {
  onSettings: (s: Settings) => void;
}

type Check = { label: string; ok: boolean | null; detail: string };

/** The end of a long path: the folder and the file, which is what identifies a model. */
function tail(path: string): string {
  const parts = path.split("/").filter(Boolean);
  return parts.length > 2 ? "…/" + parts.slice(-2).join("/") : path;
}

function checklist(st: SetupStatus): Check[] {
  const c = st.check;
  const none = (s: string) => (c ? s : "run Install engine");
  return [
    { label: "Claude CLI", ok: !!st.claude, detail: st.claude ?? "not found on the login-shell PATH" },
    { label: "uv", ok: !!st.uv, detail: st.uv ?? "not found; Install engine fetches it" },
    { label: "ffmpeg", ok: !!(st.ffmpeg ?? c?.ffmpeg), detail: st.ffmpeg ?? c?.ffmpeg ?? "brew install ffmpeg" },
    { label: "Engine venv", ok: st.venv, detail: st.venv ? `python ${c?.python ?? ""}`.trim() : "not created yet" },
    { label: "torch + device", ok: c ? !!c.torch : null, detail: c ? `${c.torch ?? "missing"}${c.device ? ` on ${c.device}` : ""}` : none("") },
    { label: "ACE-Step", ok: c ? c.acestep : null, detail: c ? (c.acestep ? "installed" : "missing") : none("") },
    { label: "demucs", ok: c ? c.demucs : null, detail: c ? (c.demucs ? "installed" : "missing") : none("") },
    { label: "rvc-python", ok: c ? c.rvc : null, detail: c ? (c.rvc ? "installed" : "missing; voice conversion is skipped") : none("") },
    { label: "ACE-Step weights", ok: c ? c.models.acestep : null, detail: c ? (c.models.acestep ? "present" : "download on first render") : none("") },
    { label: "demucs weights", ok: c ? c.models.demucs : null, detail: c ? (c.models.demucs ? "present" : "download on first separate") : none("") },
    { label: "RVC assets", ok: c ? c.models.rvcAssets : null, detail: c ? (c.models.rvcAssets ? "present" : "download on first convert") : none("") },
  ];
}

export default function Setup({ onSettings }: Props) {
  const [status, setStatus] = useState<SetupStatus | null>(null);
  const [saved, setSaved] = useState<Settings | null>(null);
  const [draft, setDraft] = useState<Settings | null>(null);
  const [log, setLog] = useState<LogEvent[]>([]);
  const [installing, setInstalling] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [note, setNote] = useState<string | null>(null);
  const logBox = useRef<HTMLPreElement>(null);

  const refresh = () => setupStatus().then(setStatus).catch((e) => setError(errText(e)));

  useEffect(() => {
    void refresh();
    getSettings()
      .then((s) => {
        setSaved(s);
        setDraft(s);
      })
      .catch((e) => setError(errText(e)));
    let off: (() => void) | undefined;
    onLog((e) => setLog((prev) => [...prev.slice(-500), e])).then((fn) => (off = fn));
    return () => off?.();
  }, []);

  // An install started by an earlier visit (or another window) shows its log too.
  useEffect(() => {
    if (status?.installing) setInstalling(true);
  }, [status?.installing]);

  useEffect(() => {
    const el = logBox.current;
    if (el) el.scrollTop = el.scrollHeight;
  }, [log.length]);

  // While something else is installing, poll so the checklist catches up.
  useEffect(() => {
    if (!status?.installing) return;
    const t = setInterval(() => void refresh(), 2000);
    return () => clearInterval(t);
  }, [status?.installing]);

  useEffect(() => {
    if (status && !status.installing && installing && !busy) setInstalling(false);
  }, [status, installing, busy]);

  const install = async () => {
    setBusy(true);
    setInstalling(true);
    setError(null);
    setLog([]);
    try {
      await installEngine();
      setNote("Engine installed.");
    } catch (e) {
      setError(errText(e));
    } finally {
      setBusy(false);
      setInstalling(false);
      void refresh();
    }
  };

  const save = async () => {
    if (!draft) return;
    setBusy(true);
    setError(null);
    try {
      const s = await setSettings(draft);
      setSaved(s);
      setDraft(s);
      onSettings(s);
      setNote("Settings saved.");
    } catch (e) {
      setError(errText(e));
    } finally {
      setBusy(false);
    }
  };

  const pick = async (kind: "pth" | "index") => {
    try {
      const path = await pickFile(kind);
      if (path && draft) setDraft({ ...draft, voiceModel: { ...draft.voiceModel, [kind]: path } });
    } catch (e) {
      setError(errText(e));
    }
  };

  const set = <K extends keyof Settings>(key: K, value: Settings[K]) => {
    if (!draft) return;
    setDraft({ ...draft, [key]: value });
    setNote(null);
  };
  const setVoice = <K extends keyof VoiceModel>(key: K, value: VoiceModel[K]) => {
    if (!draft) return;
    setDraft({ ...draft, voiceModel: { ...draft.voiceModel, [key]: value } });
    setNote(null);
  };
  const num = (e: React.ChangeEvent<HTMLInputElement>, fallback: number) => {
    const v = parseFloat(e.target.value);
    return isNaN(v) ? fallback : v;
  };

  const dirty = !!draft && !!saved && JSON.stringify(draft) !== JSON.stringify(saved);
  const items = status ? checklist(status) : [];
  const missing = items.filter((i) => i.ok === false).length;

  return (
    <section className="setup">
      <header className="page-head">
        <h1 className="display">Setup</h1>
        {status ? <span className={`dim num ${missing ? "warn" : ""}`}>{missing ? `${missing} missing` : "everything present"}</span> : null}
      </header>
      {error ? (
        <p className="error" role="alert">
          {error}
        </p>
      ) : null}
      <div className="setup-grid">
        <div className="setup-col">
          <h2 className="label">Installed</h2>
          {status ? (
            <ul className="checklist">
              {items.map((i) => (
                <li key={i.label} className={i.ok === true ? "ok" : i.ok === false ? "missing" : "unknown"}>
                  <span className="check-icon" aria-hidden="true">
                    {i.ok === true ? (
                      <svg viewBox="0 0 16 16" width="14" height="14">
                        <path d="M3 8.5l3.2 3L13 4.5" stroke="currentColor" strokeWidth="1.8" fill="none" />
                      </svg>
                    ) : i.ok === false ? (
                      <svg viewBox="0 0 16 16" width="14" height="14">
                        <path d="M4 4l8 8M12 4l-8 8" stroke="currentColor" strokeWidth="1.8" fill="none" />
                      </svg>
                    ) : (
                      <span className="dot" />
                    )}
                  </span>
                  <span className="check-label">{i.label}</span>
                  <span className="check-detail">{i.detail}</span>
                  <span className="sr-only">{i.ok === true ? "present" : i.ok === false ? "missing" : "unknown"}</span>
                </li>
              ))}
            </ul>
          ) : (
            <p className="dim">Checking</p>
          )}
          {status?.check?.problems.length ? (
            <ul className="problems">
              {status.check.problems.map((p, i) => (
                <li key={i}>{p}</li>
              ))}
            </ul>
          ) : null}
          <div className="btn-row">
            <button type="button" className={`btn primary ${installing ? "busy" : ""}`} disabled={installing || status?.installing} onClick={() => void install()}>
              {installing || status?.installing ? "Installing" : status?.venv ? "Install engine again" : "Install engine"}
            </button>
            <button type="button" className="btn quiet" onClick={() => void refresh()} disabled={installing}>
              Check again
            </button>
            <button type="button" className="btn quiet" disabled={!status} onClick={() => status && void openDataDir(status.dataDir)} title={status?.dataDir}>
              Open data folder
            </button>
          </div>
          {installing || log.length ? (
            <pre className="log" ref={logBox} aria-live="polite" aria-label="Install log">
              {log.length ? log.map((l) => l.line).join("\n") : "Starting setup.sh"}
            </pre>
          ) : null}
          {status ? (
            <dl className="paths">
              <div>
                <dt>Data folder</dt>
                <dd>{status.dataDir}</dd>
              </div>
              <div>
                <dt>Engine source</dt>
                <dd>{status.engineSource}</dd>
              </div>
            </dl>
          ) : null}
        </div>

        {draft ? (
          <form
            className="setup-col form"
            onSubmit={(e) => {
              e.preventDefault();
              void save();
            }}
          >
            <h2 className="label">Engine</h2>
            <div className="chips" role="group" aria-label="Engine">
              <button type="button" className={`chip ${draft.engine === "local" ? "on" : ""}`} aria-pressed={draft.engine === "local"} onClick={() => set("engine", "local")}>
                Local (ACE-Step on this Mac)
              </button>
              <button type="button" className={`chip ${draft.engine === "fal" ? "on" : ""}`} aria-pressed={draft.engine === "fal"} onClick={() => set("engine", "fal")}>
                fal.ai
              </button>
            </div>
            {draft.engine === "fal" ? (
              <label className="field">
                <span className="label">fal key</span>
                <input className="input mono" type="password" autoComplete="off" value={draft.falKey} onChange={(e) => set("falKey", e.target.value)} placeholder="key id:secret" />
                <span className="hint">Only the render stage runs on fal; separate, convert and mix still run here.</span>
              </label>
            ) : (
              <label className="field">
                <span className="label">Device</span>
                <select className="input" value={draft.device} onChange={(e) => set("device", e.target.value as Settings["device"])}>
                  <option value="auto">auto (mps if available)</option>
                  <option value="mps">mps</option>
                  <option value="cpu">cpu</option>
                </select>
              </label>
            )}

            <h2 className="label">Render quality</h2>
            <div className="field-grid">
              <label className="field">
                <span className="label">Steps</span>
                <input className="input num" type="number" min={1} max={200} value={draft.steps} onChange={(e) => set("steps", Math.round(num(e, 60)))} />
                <span className="hint">
                  <button type="button" className="link" onClick={() => set("steps", 27)}>
                    27 fast
                  </button>
                  {" · "}
                  <button type="button" className="link" onClick={() => set("steps", 60)}>
                    60 quality
                  </button>
                </span>
              </label>
              <label className="field">
                <span className="label">Guidance</span>
                <input className="input num" type="number" step={0.5} min={1} max={30} value={draft.guidance} onChange={(e) => set("guidance", num(e, 15))} />
              </label>
              <label className="field">
                <span className="label">Default length (s)</span>
                <input className="input num" type="number" step={30} min={30} max={360} value={draft.defaultDurationSec} onChange={(e) => set("defaultDurationSec", Math.round(num(e, 150)))} />
              </label>
            </div>

            <h2 className="label">Voice conversion</h2>
            <label className="toggle">
              <input type="checkbox" checked={draft.convert} onChange={(e) => set("convert", e.target.checked)} />
              <span>Re-sing the vocal with an RVC model</span>
            </label>
            <p className="hint">The app runs whatever .pth you point it at. It never downloads or trains one.</p>
            <fieldset className="voice" disabled={!draft.convert}>
              <div className="field">
                <span className="label">Model (.pth)</span>
                <div className="pick-row">
                  <span className="path mono" title={draft.voiceModel.pth}>
                    {draft.voiceModel.pth ? tail(draft.voiceModel.pth) : "none picked"}
                  </span>
                  <button type="button" className="btn small" onClick={() => void pick("pth")}>
                    Choose
                  </button>
                  {draft.voiceModel.pth ? (
                    <button type="button" className="icon-btn" aria-label="Clear model" onClick={() => setVoice("pth", "")}>
                      <svg viewBox="0 0 16 16" width="12" height="12" aria-hidden="true">
                        <path d="M3 3l10 10M13 3L3 13" stroke="currentColor" strokeWidth="1.8" fill="none" />
                      </svg>
                    </button>
                  ) : null}
                </div>
              </div>
              <div className="field">
                <span className="label">Index (optional)</span>
                <div className="pick-row">
                  <span className="path mono" title={draft.voiceModel.index}>
                    {draft.voiceModel.index ? tail(draft.voiceModel.index) : "none"}
                  </span>
                  <button type="button" className="btn small" onClick={() => void pick("index")}>
                    Choose
                  </button>
                  {draft.voiceModel.index ? (
                    <button type="button" className="icon-btn" aria-label="Clear index" onClick={() => setVoice("index", "")}>
                      <svg viewBox="0 0 16 16" width="12" height="12" aria-hidden="true">
                        <path d="M3 3l10 10M13 3L3 13" stroke="currentColor" strokeWidth="1.8" fill="none" />
                      </svg>
                    </button>
                  ) : null}
                </div>
              </div>
              <div className="field-grid">
                <label className="field">
                  <span className="label">Pitch (st)</span>
                  <input className="input num" type="number" min={-24} max={24} value={draft.voiceModel.pitch} onChange={(e) => setVoice("pitch", Math.round(num(e, 0)))} />
                </label>
                <label className="field">
                  <span className="label">Method</span>
                  <select className="input" value={draft.voiceModel.method} onChange={(e) => setVoice("method", e.target.value as VoiceModel["method"])}>
                    <option value="rmvpe">rmvpe</option>
                    <option value="harvest">harvest</option>
                    <option value="crepe">crepe</option>
                    <option value="pm">pm</option>
                  </select>
                </label>
                <label className="field">
                  <span className="label">Index rate</span>
                  <input className="input num" type="number" step={0.01} min={0} max={1} value={draft.voiceModel.indexRate} onChange={(e) => setVoice("indexRate", num(e, 0.66))} />
                </label>
                <label className="field">
                  <span className="label">Protect</span>
                  <input className="input num" type="number" step={0.01} min={0} max={0.5} value={draft.voiceModel.protect} onChange={(e) => setVoice("protect", num(e, 0.33))} />
                </label>
              </div>
            </fieldset>

            <h2 className="label">Mix</h2>
            <div className="field-grid">
              <label className="field">
                <span className="label">Vocal gain (dB)</span>
                <input className="input num" type="number" step={0.5} min={-12} max={12} value={draft.vocalsGainDb} onChange={(e) => set("vocalsGainDb", num(e, 0))} />
                <span className="hint">Vocal level against the beat.</span>
              </label>
              <label className="field">
                <span className="label">mp3 bitrate</span>
                <select className="input" value={draft.mp3Bitrate} onChange={(e) => set("mp3Bitrate", parseInt(e.target.value, 10))}>
                  <option value={192}>192k</option>
                  <option value={256}>256k</option>
                  <option value={320}>320k</option>
                </select>
              </label>
            </div>

            <h2 className="label">Writing</h2>
            <div className="field-grid">
              <label className="field">
                <span className="label">Claude model</span>
                <select className="input" value={draft.claudeModel} onChange={(e) => set("claudeModel", e.target.value)}>
                  <option value="">sonnet</option>
                  <option value="opus">opus</option>
                  <option value="default">CLI default</option>
                </select>
              </label>
              <label className="field">
                <span className="label">Effort</span>
                <select className="input" value={draft.claudeEffort} onChange={(e) => set("claudeEffort", e.target.value as Settings["claudeEffort"])}>
                  <option value="low">low</option>
                  <option value="medium">medium</option>
                  <option value="high">high</option>
                  <option value="default">default</option>
                </select>
              </label>
            </div>

            <div className="btn-row sticky-actions">
              <button type="submit" className="btn primary" disabled={!dirty || busy}>
                Save settings
              </button>
              <button type="button" className="btn quiet" disabled={!dirty || busy} onClick={() => setDraft(saved)}>
                Discard
              </button>
              {note ? <span className="hint">{note}</span> : dirty ? <span className="hint">Unsaved changes</span> : null}
            </div>
          </form>
        ) : null}
      </div>
    </section>
  );
}
