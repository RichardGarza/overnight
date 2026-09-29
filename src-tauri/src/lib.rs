//! Overnight: the app shell. One song goes through the lyrics stage (the
//! Claude CLI writes title, tags and lyrics, lyrics.rs), then render (ACE-Step
//! in the venv, or fal.ai) and separate, convert, mix (the Python engine,
//! engine.rs). Progress reaches the UI as `overnight-progress` events; the installer's
//! output as `overnight-log`. The modules do the work; this file wires the
//! commands of SPEC §4.1 to them and holds the little state there is.

pub mod engine;
pub mod lyrics;
pub mod model;
pub mod paths;
pub mod store;

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use rand::Rng;
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_opener::OpenerExt;

use model::{LogEvent, Persona, ProgressEvent, Settings, SetupStatus, Song, SongPatch, DONE, PENDING, SKIPPED};

struct AppState {
    /// One render at a time: the machine has one GPU and 18 GB.
    rendering: AtomicBool,
    /// The render in flight, so cancel_render can find it.
    current: Mutex<Option<(String, engine::Cancel)>>,
    installing: AtomicBool,
}

impl Default for AppState {
    fn default() -> Self {
        Self { rendering: AtomicBool::new(false), current: Mutex::new(None), installing: AtomicBool::new(false) }
    }
}

/// Clears the render flag however the render ends, including a panic.
struct RenderGuard<'a>(&'a AppState);

impl Drop for RenderGuard<'_> {
    fn drop(&mut self) {
        self.0.rendering.store(false, Ordering::SeqCst);
        if let Ok(mut c) = self.0.current.lock() {
            *c = None;
        }
    }
}

fn app_data(app: &AppHandle) -> Result<PathBuf, String> {
    store::data_dir(app)
}

fn emit_progress(app: &AppHandle, ev: ProgressEvent) {
    let _ = app.emit("overnight-progress", ev);
}

fn emit_log(app: &AppHandle, line: String) {
    let _ = app.emit("overnight-log", LogEvent { line, at: model::now_ms() });
}

/// Folders and PATH for the engine runner.
async fn engine_ctx(app: &AppHandle) -> Result<engine::Ctx, String> {
    let data_dir = app_data(app)?;
    let engine_dir = engine::locate_engine(app.path().resource_dir().ok().as_deref())?;
    let path = paths::shell_env().await.path.clone();
    Ok(engine::Ctx { data_dir, engine_dir, path })
}

// ----------------------------------------------------------------- settings

#[tauri::command]
fn get_settings(app: AppHandle) -> Result<Settings, String> {
    Ok(store::load_settings(&app_data(&app)?))
}

#[tauri::command]
fn set_settings(app: AppHandle, settings: Settings) -> Result<Settings, String> {
    let dir = app_data(&app)?;
    let mut settings = settings;
    tidy_settings(&mut settings);
    store::save_settings(&dir, &settings)?;
    Ok(store::load_settings(&dir))
}

/// Keep the enum-ish strings to their known values so the engine never sees
/// a typo, and trim the paths.
fn tidy_settings(s: &mut Settings) {
    if !matches!(s.engine.as_str(), "local" | "fal") {
        s.engine = "local".into();
    }
    if !matches!(s.device.as_str(), "auto" | "mps" | "cpu") {
        s.device = "auto".into();
    }
    if !matches!(s.claude_effort.as_str(), "low" | "medium" | "high" | "default") {
        s.claude_effort = "medium".into();
    }
    if !matches!(s.voice_model.method.as_str(), "rmvpe" | "harvest" | "crepe" | "pm") {
        s.voice_model.method = "rmvpe".into();
    }
    s.steps = s.steps.clamp(1, 200);
    s.default_duration_sec = s.default_duration_sec.clamp(20, 600);
    for p in [&mut s.fal_key, &mut s.claude_bin, &mut s.uv_bin, &mut s.voice_model.pth, &mut s.voice_model.index] {
        *p = p.trim().to_string();
    }
}

// ------------------------------------------------------------------ persona

#[tauri::command]
fn get_persona(app: AppHandle) -> Result<Persona, String> {
    store::load_persona(&app_data(&app)?)
}

#[tauri::command]
fn set_persona(app: AppHandle, persona: Persona) -> Result<Persona, String> {
    let dir = app_data(&app)?;
    let mut persona = persona;
    if persona.name.trim().is_empty() {
        persona.name = "Overnight".into();
    }
    store::save_persona(&dir, &persona)?;
    store::load_persona(&dir)
}

#[tauri::command]
fn reset_persona(app: AppHandle) -> Result<Persona, String> {
    let dir = app_data(&app)?;
    let persona = store::default_persona()?;
    store::save_persona(&dir, &persona)?;
    Ok(persona)
}

// -------------------------------------------------------------------- setup

#[tauri::command]
async fn setup_status(app: AppHandle, state: State<'_, AppState>) -> Result<SetupStatus, String> {
    let dir = app_data(&app)?;
    let settings = store::load_settings(&dir);
    let claude = paths::find_bin("claude", &settings.claude_bin).await;
    let uv = paths::find_bin("uv", &settings.uv_bin).await;
    let ffmpeg = paths::find_bin("ffmpeg", "").await;
    let engine_source = engine::locate_engine(app.path().resource_dir().ok().as_deref()).ok();
    let venv = engine::has_venv(&dir);
    let check = match &engine_source {
        Some(engine_dir) if venv => {
            let ctx = engine::Ctx { data_dir: dir.clone(), engine_dir: engine_dir.clone(), path: paths::shell_env().await.path.clone() };
            engine::check(&ctx, &settings).await
        }
        _ => None,
    };
    Ok(SetupStatus {
        data_dir: dir.display().to_string(),
        claude: claude.map(|p| p.display().to_string()),
        uv: uv.map(|p| p.display().to_string()),
        ffmpeg: ffmpeg.map(|p| p.display().to_string()),
        engine_source: engine_source.map(|p| p.display().to_string()),
        venv,
        check,
        installing: state.installing.load(Ordering::SeqCst),
    })
}

#[tauri::command]
async fn install_engine(app: AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    if state.installing.swap(true, Ordering::SeqCst) {
        return Err("The engine is already being installed.".into());
    }
    let result = install_engine_inner(&app).await;
    state.installing.store(false, Ordering::SeqCst);
    result
}

async fn install_engine_inner(app: &AppHandle) -> Result<(), String> {
    let ctx = engine_ctx(app).await?;
    let settings = store::load_settings(&ctx.data_dir);
    let uv = paths::find_bin("uv", &settings.uv_bin).await;
    if uv.is_none() {
        emit_log(app, "uv not found. Install it with: curl -LsSf https://astral.sh/uv/install.sh | sh".into());
    }
    emit_log(app, format!("Engine source: {}", ctx.engine_dir.display()));
    let handle = app.clone();
    let on_line = move |l: String| emit_log(&handle, l);
    let result = engine::install(&ctx, uv.as_deref(), &on_line).await;
    match &result {
        Ok(()) => emit_log(app, "Engine installed.".into()),
        Err(e) => emit_log(app, format!("Install failed: {e}")),
    }
    result
}

// -------------------------------------------------------------------- songs

#[tauri::command]
async fn write_song(app: AppHandle, theme: String, mood: String, duration_sec: u32) -> Result<Song, String> {
    let dir = app_data(&app)?;
    let theme = theme.trim().to_string();
    if theme.is_empty() {
        return Err("Say what the song is about first.".into());
    }
    let settings = store::load_settings(&dir);
    let persona = store::load_persona(&dir)?;
    let duration = if duration_sec == 0 { settings.default_duration_sec } else { duration_sec.clamp(20, 600) };
    let bin = paths::find_bin("claude", &settings.claude_bin)
        .await
        .ok_or("Claude CLI not found. Install it (npm i -g @anthropic-ai/claude-code) or set claudeBin in Setup.")?;

    // The id is made up front so the lyrics stage can report against it.
    let id = store::new_song_id();
    let progress = {
        let app = app.clone();
        let id = id.clone();
        move |event: &str, message: &str, detail: Option<String>| {
            emit_progress(&app, ProgressEvent { song_id: id.clone(), stage: "lyrics".into(), event: event.into(), pct: None, message: message.into(), detail, at: model::now_ms() });
        }
    };
    progress("start", "Claude is writing", None);
    let prompt = lyrics::build_prompt(&persona, &theme, &mood, duration);
    let reply = {
        let progress = progress.clone();
        lyrics::run(&bin, &store::work_dir(&dir), &prompt, &settings, move |msg, detail| progress("progress", msg, detail)).await
    };
    let reply = match reply {
        Ok(r) => r,
        Err(e) => {
            progress("error", &e, None);
            return Err(e);
        }
    };
    let out = match lyrics::parse_reply(&reply, &persona, &theme, duration) {
        Ok(o) => o,
        Err(e) => {
            progress("error", &e, None);
            return Err(e);
        }
    };

    let convert_on = settings.convert && !settings.voice_model.pth.trim().is_empty();
    let stages = model::Stages { lyrics: DONE.into(), convert: if convert_on { PENDING.into() } else { SKIPPED.into() }, ..Default::default() };
    let song = Song {
        id: id.clone(),
        title: out.title,
        theme,
        mood: mood.trim().to_string(),
        created_at: chrono::Local::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, false),
        persona,
        lyrics: out.lyrics,
        tags: out.tags,
        bpm: out.bpm,
        key: out.key,
        duration_sec: duration,
        engine: if settings.engine == "fal" { "fal".into() } else { "local".into() },
        // Under 2^31 so the Python side and JavaScript both take it as a plain int.
        seed: rand::thread_rng().gen_range(1..2_147_483_647u64),
        steps: settings.steps,
        stages,
        error: None,
        files: Default::default(),
        actual_duration_sec: None,
        render_seconds: None,
    };
    store::save_song(&dir, &song)?;
    store::save_lyrics_txt(&dir, &song)?;
    let _ = std::fs::write(store::song_dir(&dir, &id)?.join("cover_mood.txt"), format!("{}\n", out.cover_mood));
    progress("done", &format!("\u{201c}{}\u{201d}", song.title), None);
    Ok(song)
}

#[tauri::command]
fn update_song(app: AppHandle, id: String, patch: SongPatch) -> Result<Song, String> {
    let dir = app_data(&app)?;
    let mut song = store::load_song(&dir, &id)?;
    if let Some(t) = patch.title {
        let t = t.trim();
        if !t.is_empty() {
            song.title = lyrics::truncate(t, 120);
        }
    }
    if let Some(l) = patch.lyrics {
        let tidy = lyrics::tidy_lyrics(&l);
        if !tidy.trim().is_empty() {
            song.lyrics = tidy;
        }
    }
    if let Some(t) = patch.tags {
        let t = t.trim();
        if !t.is_empty() {
            song.tags = t.to_string();
        }
    }
    if let Some(d) = patch.duration_sec {
        song.duration_sec = d.clamp(20, 600);
    }
    store::save_song(&dir, &song)?;
    store::save_lyrics_txt(&dir, &song)?;
    Ok(song)
}

#[tauri::command]
async fn render_song(app: AppHandle, state: State<'_, AppState>, id: String, from: String) -> Result<Song, String> {
    if !store::is_valid_song_id(&id) {
        return Err(format!("not a song id: {id:?}"));
    }
    if state.rendering.swap(true, Ordering::SeqCst) {
        return Err("Another render is already running. Wait for it or cancel it first.".into());
    }
    let _guard = RenderGuard(&state);
    let cancel = engine::Cancel::new();
    if let Ok(mut c) = state.current.lock() {
        *c = Some((id.clone(), cancel.clone()));
    }
    let ctx = engine_ctx(&app).await?;
    let handle = app.clone();
    let sink = move |ev: ProgressEvent| emit_progress(&handle, ev);
    engine::run_pipeline(&ctx, &id, &from, &sink, &cancel).await
}

#[tauri::command]
fn cancel_render(state: State<'_, AppState>, id: String) -> Result<(), String> {
    let current = state.current.lock().map_err(|_| "state poisoned")?;
    match current.as_ref() {
        Some((running, cancel)) if *running == id => {
            cancel.cancel();
            Ok(())
        }
        Some((running, _)) => Err(format!("That song isn't rendering ({running} is).")),
        None => Ok(()),
    }
}

#[tauri::command]
fn list_songs(app: AppHandle) -> Result<Vec<Song>, String> {
    Ok(store::list_songs(&app_data(&app)?))
}

#[tauri::command]
fn get_song(app: AppHandle, id: String) -> Result<Song, String> {
    store::load_song(&app_data(&app)?, &id)
}

#[tauri::command]
fn delete_song(app: AppHandle, state: State<'_, AppState>, id: String) -> Result<(), String> {
    if let Ok(current) = state.current.lock() {
        if matches!(current.as_ref(), Some((running, _)) if *running == id) {
            return Err("That song is rendering. Cancel it first.".into());
        }
    }
    store::delete_song(&app_data(&app)?, &id)
}

#[tauri::command]
fn reveal_song(app: AppHandle, id: String) -> Result<(), String> {
    let folder = store::song_dir(&app_data(&app)?, &id)?;
    if !folder.is_dir() {
        return Err("That song's folder is gone.".into());
    }
    app.opener().open_path(folder.display().to_string(), None::<&str>).map_err(|e| format!("open folder: {e}"))
}

/// Native open-file dialog for the voice model files. "" when dismissed.
#[tauri::command]
async fn pick_file(app: AppHandle, kind: String) -> Result<String, String> {
    let (label, ext) = match kind.as_str() {
        "pth" => ("Voice model", "pth"),
        "index" => ("Voice index", "index"),
        other => return Err(format!("unknown file kind: {other}")),
    };
    let start = app_data(&app)?.join("voices");
    let picked = tauri::async_runtime::spawn_blocking(move || {
        let mut dialog = app.dialog().file().add_filter(label, &[ext]).set_title(format!("Choose the .{ext} file"));
        if start.is_dir() {
            dialog = dialog.set_directory(&start);
        }
        dialog.blocking_pick_file()
    })
    .await
    .map_err(|e| format!("dialog: {e}"))?;
    Ok(picked.and_then(|p| p.into_path().ok()).map(|p| p.display().to_string()).unwrap_or_default())
}

#[tauri::command]
fn data_dir(app: AppHandle) -> Result<String, String> {
    Ok(app_data(&app)?.display().to_string())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .manage(AppState::default())
        .invoke_handler(tauri::generate_handler![
            get_settings,
            set_settings,
            get_persona,
            set_persona,
            reset_persona,
            setup_status,
            install_engine,
            write_song,
            update_song,
            render_song,
            cancel_render,
            list_songs,
            get_song,
            delete_song,
            reveal_song,
            pick_file,
            data_dir,
        ])
        .setup(|app| {
            // Make the folders on first launch so Setup can show the path.
            let _ = store::data_dir(app.handle());
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running Overnight");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_are_tidied_to_known_values() {
        let mut s = Settings { engine: "cloud".into(), device: "cuda".into(), claude_effort: "max".into(), steps: 0, ..Default::default() };
        s.voice_model.method = "magic".into();
        s.voice_model.pth = "  /a/b.pth ".into();
        tidy_settings(&mut s);
        assert_eq!(s.engine, "local");
        assert_eq!(s.device, "auto");
        assert_eq!(s.claude_effort, "medium");
        assert_eq!(s.voice_model.method, "rmvpe");
        assert_eq!(s.steps, 1);
        assert_eq!(s.voice_model.pth, "/a/b.pth");
    }
}
