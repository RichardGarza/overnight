//! Everything on disk: settings.json, persona.json, songs/<id>/song.json.
//! All of it lives in the app data folder
//! (macOS: ~/Library/Application Support/com.overnight.studio), or wherever
//! OVERNIGHT_DATA_DIR points, which is how the tests get a scratch folder.
//!
//! Functions here take the data dir as a plain path rather than the Tauri
//! app handle, so the same code runs in integration tests without a window.

use std::fs;
use std::path::{Path, PathBuf};

use rand::Rng;
use tauri::{AppHandle, Manager};

use crate::model::{Persona, Settings, Song};

const DEFAULT_PERSONA: &str = include_str!("../resources/default-persona.json");

/// Env var that replaces the app data folder (tests, or a second library).
pub const DATA_DIR_ENV: &str = "OVERNIGHT_DATA_DIR";

/// The app data folder, with its subfolders created.
pub fn data_dir(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = match std::env::var_os(DATA_DIR_ENV).filter(|v| !v.is_empty()) {
        Some(v) => PathBuf::from(v),
        None => app.path().app_data_dir().map_err(|e| format!("no app data dir: {e}"))?,
    };
    ensure_layout(&dir)?;
    Ok(dir)
}

/// Create the folders of SPEC §1 that the app itself owns. The venv and the
/// model weights are the installer's business.
pub fn ensure_layout(dir: &Path) -> Result<(), String> {
    for sub in ["songs", "workdir", "voices", "models"] {
        fs::create_dir_all(dir.join(sub)).map_err(|e| format!("create {}: {e}", dir.join(sub).display()))?;
    }
    Ok(())
}

/// Empty scratch folder Claude runs in, so it never picks up a project's
/// CLAUDE.md or touches real files.
pub fn work_dir(dir: &Path) -> PathBuf {
    dir.join("workdir")
}

pub fn songs_dir(dir: &Path) -> PathBuf {
    dir.join("songs")
}

pub fn venv_dir(dir: &Path) -> PathBuf {
    dir.join("venv")
}

pub fn models_dir(dir: &Path) -> PathBuf {
    dir.join("models")
}

// ----------------------------------------------------------------- settings

pub fn load_settings(dir: &Path) -> Settings {
    let path = dir.join("settings.json");
    match fs::read_to_string(&path) {
        Ok(text) => serde_json::from_str(&text).unwrap_or_else(|e| {
            // Keep the broken file for inspection, fall back to defaults.
            eprintln!("settings.json unreadable ({e}); using defaults");
            let _ = fs::copy(&path, path.with_extension("json.broken"));
            Settings::default()
        }),
        Err(_) => {
            // Write the defaults once so the knobs are discoverable.
            if let Ok(text) = serde_json::to_string_pretty(&Settings::default()) {
                let _ = fs::write(&path, text);
            }
            Settings::default()
        }
    }
}

pub fn save_settings(dir: &Path, settings: &Settings) -> Result<(), String> {
    let text = serde_json::to_string_pretty(settings).map_err(|e| e.to_string())?;
    write_atomic(&dir.join("settings.json"), &text)
}

/// Read-modify-write, so a switch in the UI never clobbers hand edits.
pub fn update_settings(dir: &Path, change: impl FnOnce(&mut Settings)) -> Result<Settings, String> {
    let mut settings = load_settings(dir);
    change(&mut settings);
    save_settings(dir, &settings)?;
    Ok(settings)
}

// ------------------------------------------------------------------ persona

pub fn default_persona() -> Result<Persona, String> {
    serde_json::from_str(DEFAULT_PERSONA).map_err(|e| format!("default persona: {e}"))
}

pub fn load_persona(dir: &Path) -> Result<Persona, String> {
    let path = dir.join("persona.json");
    if !path.exists() {
        write_atomic(&path, DEFAULT_PERSONA.trim_end()).map_err(|e| format!("seed persona: {e}"))?;
    }
    let text = fs::read_to_string(&path).map_err(|e| format!("read persona: {e}"))?;
    match serde_json::from_str::<Persona>(&text) {
        Ok(p) => Ok(p),
        Err(e) => {
            let _ = fs::copy(&path, path.with_extension("json.broken"));
            eprintln!("persona.json unreadable ({e}); using the default");
            default_persona()
        }
    }
}

pub fn save_persona(dir: &Path, persona: &Persona) -> Result<(), String> {
    let text = serde_json::to_string_pretty(persona).map_err(|e| e.to_string())?;
    write_atomic(&dir.join("persona.json"), &text)
}

// -------------------------------------------------------------------- songs

/// `YYYYMMDD-HHMMSS-xxxx`: local time plus four random lowercase alphanumerics,
/// so ids sort by creation time and two songs in one second still differ.
pub fn new_song_id() -> String {
    const ALPHABET: &[u8] = b"abcdefghijklmnopqrstuvwxyz0123456789";
    let mut rng = rand::thread_rng();
    let tail: String = (0..4).map(|_| ALPHABET[rng.gen_range(0..ALPHABET.len())] as char).collect();
    format!("{}-{tail}", chrono::Local::now().format("%Y%m%d-%H%M%S"))
}

/// Song ids are used as folder names, so anything that is not one is
/// rejected before it can reach the file system.
pub fn is_valid_song_id(id: &str) -> bool {
    let b = id.as_bytes();
    if b.len() != 20 || b[8] != b'-' || b[15] != b'-' {
        return false;
    }
    b[..8].iter().chain(&b[9..15]).all(|c| c.is_ascii_digit())
        && b[16..].iter().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
}

pub fn song_dir(dir: &Path, id: &str) -> Result<PathBuf, String> {
    if !is_valid_song_id(id) {
        return Err(format!("not a song id: {id:?}"));
    }
    Ok(songs_dir(dir).join(id))
}

pub fn load_song(dir: &Path, id: &str) -> Result<Song, String> {
    let path = song_dir(dir, id)?.join("song.json");
    let text = fs::read_to_string(&path).map_err(|e| format!("read {}: {e}", path.display()))?;
    serde_json::from_str(&text).map_err(|e| format!("song.json of {id}: {e}"))
}

pub fn save_song(dir: &Path, song: &Song) -> Result<(), String> {
    let folder = song_dir(dir, &song.id)?;
    fs::create_dir_all(&folder).map_err(|e| format!("create {}: {e}", folder.display()))?;
    let text = serde_json::to_string_pretty(song).map_err(|e| e.to_string())?;
    write_atomic(&folder.join("song.json"), &text)
}

/// The lyrics as sent to the model, kept next to song.json for humans.
pub fn save_lyrics_txt(dir: &Path, song: &Song) -> Result<(), String> {
    let folder = song_dir(dir, &song.id)?;
    fs::create_dir_all(&folder).map_err(|e| format!("create {}: {e}", folder.display()))?;
    let mut text = song.lyrics.trim_end().to_string();
    text.push('\n');
    write_atomic(&folder.join("lyrics.txt"), &text)
}

/// Newest first. A folder without a readable song.json is skipped, not fatal.
pub fn list_songs(dir: &Path) -> Vec<Song> {
    let mut ids: Vec<String> = fs::read_dir(songs_dir(dir))
        .map(|rd| {
            rd.flatten()
                .filter(|e| e.path().is_dir())
                .map(|e| e.file_name().to_string_lossy().to_string())
                .filter(|n| is_valid_song_id(n))
                .collect()
        })
        .unwrap_or_default();
    ids.sort();
    ids.reverse();
    ids.iter().filter_map(|id| load_song(dir, id).ok()).collect()
}

pub fn delete_song(dir: &Path, id: &str) -> Result<(), String> {
    let folder = song_dir(dir, id)?;
    if !folder.exists() {
        return Ok(());
    }
    fs::remove_dir_all(&folder).map_err(|e| format!("delete {}: {e}", folder.display()))
}

/// Write to a sibling temp file and rename over the target, so a crash mid-write
/// leaves the old file intact instead of a truncated one.
pub fn write_atomic(path: &Path, text: &str) -> Result<(), String> {
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, text).map_err(|e| format!("write {}: {e}", tmp.display()))?;
    fs::rename(&tmp, path).map_err(|e| format!("rename {}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("overnight-store-{name}-{}-{}", std::process::id(), crate::model::now_ms()));
        let _ = fs::remove_dir_all(&dir);
        ensure_layout(&dir).unwrap();
        dir
    }

    #[test]
    fn song_ids_have_the_spec_shape() {
        let id = new_song_id();
        assert!(is_valid_song_id(&id), "{id}");
        assert_eq!(id.len(), 20);
        assert!(is_valid_song_id("20260929-153000-k3q9"));
        assert!(!is_valid_song_id("20260929-153000-K3Q9"));
        assert!(!is_valid_song_id("../../etc"));
        assert!(!is_valid_song_id("20260929-153000-k3q"));
        assert!(!is_valid_song_id(""));
        // Ids made back to back sort by time.
        let a = new_song_id();
        let b = new_song_id();
        assert!(a[..15] <= b[..15]);
    }

    #[test]
    fn settings_round_trip_and_read_modify_write() {
        let dir = scratch("settings");
        let s = load_settings(&dir);
        assert_eq!(s, Settings::default());
        assert!(dir.join("settings.json").exists(), "defaults are written out");
        // A hand edit survives a save from the UI that only touches one field.
        let mut hand = load_settings(&dir);
        hand.fal_key = "secret".into();
        save_settings(&dir, &hand).unwrap();
        let after = update_settings(&dir, |s| s.steps = 27).unwrap();
        assert_eq!(after.steps, 27);
        assert_eq!(after.fal_key, "secret");
        assert!(!dir.join("settings.tmp").exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn broken_settings_fall_back_to_defaults() {
        let dir = scratch("broken");
        fs::write(dir.join("settings.json"), "{not json").unwrap();
        assert_eq!(load_settings(&dir), Settings::default());
        assert!(dir.join("settings.json.broken").exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn persona_seeds_from_the_default_and_resets() {
        let dir = scratch("persona");
        let p = load_persona(&dir).unwrap();
        assert_eq!(p.name, "Overnight");
        assert!(p.tags.starts_with("melodic rap"));
        assert_eq!(p, default_persona().unwrap());
        let mut edited = p.clone();
        edited.name = "Nobody".into();
        save_persona(&dir, &edited).unwrap();
        assert_eq!(load_persona(&dir).unwrap().name, "Nobody");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn songs_save_list_newest_first_and_delete() {
        let dir = scratch("songs");
        let mk = |id: &str| Song { id: id.into(), title: id.into(), lyrics: "[verse]\nhello".into(), ..Default::default() };
        for id in ["20260101-000000-aaaa", "20260301-000000-cccc", "20260201-000000-bbbb"] {
            let s = mk(id);
            save_song(&dir, &s).unwrap();
            save_lyrics_txt(&dir, &s).unwrap();
        }
        fs::create_dir_all(dir.join("songs/not-a-song")).unwrap();
        let ids: Vec<String> = list_songs(&dir).into_iter().map(|s| s.id).collect();
        assert_eq!(ids, vec!["20260301-000000-cccc", "20260201-000000-bbbb", "20260101-000000-aaaa"]);
        assert_eq!(fs::read_to_string(dir.join("songs/20260101-000000-aaaa/lyrics.txt")).unwrap(), "[verse]\nhello\n");
        assert!(save_song(&dir, &mk("../escape")).is_err());
        assert!(load_song(&dir, "nope").is_err());
        delete_song(&dir, "20260201-000000-bbbb").unwrap();
        delete_song(&dir, "20260201-000000-bbbb").unwrap(); // already gone is fine
        assert_eq!(list_songs(&dir).len(), 2);
        let _ = fs::remove_dir_all(&dir);
    }
}
