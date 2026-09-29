// The shell: a top nav and one page at a time. Settings, the persona and the
// data folder are loaded once here; the Studio's flow lives in useStudio so
// a render keeps going while another tab is open.

import { useEffect, useState } from "react";
import type { Persona, Settings } from "./types";
import { dataDir as fetchDataDir, getPersona, getSettings, inTauri } from "./api";
import { useStudio } from "./studio";
import Studio from "./components/Studio";
import Library from "./components/Library";
import Artist from "./components/Artist";
import Setup from "./components/Setup";

type View = "studio" | "library" | "artist" | "setup";

const TABS: Array<{ id: View; label: string }> = [
  { id: "studio", label: "Studio" },
  { id: "library", label: "Library" },
  { id: "artist", label: "Artist" },
  { id: "setup", label: "Setup" },
];

function initialView(): View {
  const v = new URLSearchParams(window.location.search).get("view");
  return TABS.some((t) => t.id === v) ? (v as View) : "studio";
}

export default function App() {
  const [view, setView] = useState<View>(initialView);
  const [dataDir, setDataDir] = useState("");
  const [settings, setSettings] = useState<Settings | null>(null);
  const [persona, setPersona] = useState<Persona | null>(null);
  const session = useStudio();

  useEffect(() => {
    fetchDataDir().then(setDataDir).catch(() => {});
    getSettings().then(setSettings).catch(() => {});
    getPersona().then(setPersona).catch(() => {});
  }, []);

  const busy = session.phase === "rendering" && !session.error;

  return (
    <div className="app">
      <header className="topbar">
        <div className="brand">
          <span className="brand-name">{persona?.name ?? "Overnight"}</span>
          <span className="brand-tag">{persona?.tagline ?? ""}</span>
        </div>
        <nav className="nav" aria-label="Pages">
          {TABS.map((t) => (
            <button key={t.id} type="button" className={`nav-btn ${view === t.id ? "on" : ""}`} aria-current={view === t.id ? "page" : undefined} onClick={() => setView(t.id)}>
              {t.label}
              {t.id === "studio" && busy && view !== "studio" ? <span className="pulse" aria-label="rendering" /> : null}
            </button>
          ))}
        </nav>
        {!inTauri ? <span className="preview-tag">browser preview</span> : <span />}
      </header>
      <main className={`page ${view}`}>
        {view === "studio" ? <Studio session={session} settings={settings} dataDir={dataDir} /> : null}
        {view === "library" ? (
          <Library
            dataDir={dataDir}
            live={session.live}
            version={session.version}
            onOpen={(song) => {
              session.open(song);
              setView("studio");
            }}
          />
        ) : null}
        {view === "artist" ? <Artist onChange={setPersona} /> : null}
        {view === "setup" ? <Setup onSettings={setSettings} /> : null}
      </main>
    </div>
  );
}
