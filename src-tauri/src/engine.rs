//! The engine runner: the Python pipeline in the venv, and the fal.ai
//! renderer that replaces its first stage when the user has no GPU time.
//!
//! The Python side speaks one JSON object per stdout line (SPEC §3.3). We
//! spawn `<venv>/bin/python -m overnight_engine run ...`, read those lines,
//! forward each one to the UI as an `overnight-progress` event, and keep
//! `song.json` honest about which stage is running, done or broken. Anything
//! that is not a protocol line, and everything on stderr, goes to the song's
//! `engine.log` so a failed render can be read after the fact.
//!
//! Nothing here touches the Tauri app handle: the caller hands in a `Ctx`
//! with the folders and a sink for events, which is how the integration
//! tests drive the same code against a fake engine.

use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::sync::Notify;

use crate::model::{now_ms, ProgressEvent, Settings, Song, DONE, ERROR, PENDING, RUNNING, SKIPPED};
use crate::store;

/// Env var that replaces the engine source folder (tests, or hacking on the engine).
pub const ENGINE_DIR_ENV: &str = "OVERNIGHT_ENGINE_DIR";

/// The engine stages the runner can start from, in order.
pub const RUN_STAGES: [&str; 4] = ["render", "separate", "convert", "mix"];

/// What a stage leaves behind, so bookkeeping can tell "done" from "skipped".
const OUTPUTS: [(&str, &[&str]); 4] = [
    ("render", &["raw.wav"]),
    ("separate", &["vocals.wav", "instrumental.wav"]),
    ("convert", &["vocals_converted.wav"]),
    ("mix", &["final.wav", "final.mp3", "cover.png"]),
];

/// Where things are for one run. Built once by the app, or by a test.
#[derive(Clone, Debug)]
pub struct Ctx {
    pub data_dir: PathBuf,
    pub engine_dir: PathBuf,
    /// PATH for child processes (login shell + known install dirs), so the
    /// engine finds ffmpeg the same way the terminal does.
    pub path: String,
}

/// Something that receives progress events: the Tauri emitter, or a test's Vec.
pub type Sink = dyn Fn(ProgressEvent) + Send + Sync;

// ------------------------------------------------------------------ locate

/// The engine source: the bundled resource first, else `engine/` next to
/// some ancestor of the executable (the repo, in dev), else a clear error.
pub fn locate_engine(resource_dir: Option<&Path>) -> Result<PathBuf, String> {
    let mut tried: Vec<PathBuf> = Vec::new();
    if let Some(v) = std::env::var_os(ENGINE_DIR_ENV).filter(|v| !v.is_empty()) {
        tried.push(PathBuf::from(v));
    }
    if let Some(r) = resource_dir {
        tried.push(r.join("resources").join("engine"));
        tried.push(r.join("engine"));
    }
    if let Ok(exe) = std::env::current_exe() {
        for a in exe.ancestors().skip(1) {
            tried.push(a.join("engine"));
        }
    }
    for dir in &tried {
        if is_engine_dir(dir) {
            return Ok(dir.clone());
        }
    }
    Err(format!(
        "Couldn't find the engine source (a folder with overnight_engine/ in it). Looked in: {}",
        tried.iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join(", ")
    ))
}

fn is_engine_dir(dir: &Path) -> bool {
    dir.join("overnight_engine").is_dir()
}

pub fn venv_python(data_dir: &Path) -> PathBuf {
    store::venv_dir(data_dir).join("bin").join("python")
}

pub fn has_venv(data_dir: &Path) -> bool {
    venv_python(data_dir).is_file()
}

/// "auto" means mps on Apple Silicon, cpu anywhere else.
pub fn resolve_device(setting: &str) -> &'static str {
    match setting.trim() {
        "mps" => "mps",
        "cpu" => "cpu",
        _ => {
            if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
                "mps"
            } else {
                "cpu"
            }
        }
    }
}

// ------------------------------------------------------------------ cancel

/// A flag the UI can raise from another task. `wait` resolves once it is raised.
#[derive(Clone, Default)]
pub struct Cancel(Arc<CancelInner>);

#[derive(Default)]
struct CancelInner {
    flag: AtomicBool,
    notify: Notify,
}

impl Cancel {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn cancel(&self) {
        self.0.flag.store(true, Ordering::SeqCst);
        self.0.notify.notify_waiters();
        self.0.notify.notify_one();
    }
    pub fn is_cancelled(&self) -> bool {
        self.0.flag.load(Ordering::SeqCst)
    }
    pub async fn wait(&self) {
        while !self.is_cancelled() {
            self.0.notify.notified().await;
        }
    }
}

// ---------------------------------------------------------------- protocol

/// One line of the progress protocol (SPEC §3.3).
#[derive(Debug, Clone, PartialEq)]
pub struct ProgressLine {
    pub stage: String,
    pub event: String,
    pub pct: Option<u32>,
    pub message: String,
    pub detail: Option<String>,
    pub at: i64,
}

/// None for anything that is not a protocol line (a library's stray print).
pub fn parse_progress_line(line: &str) -> Option<ProgressLine> {
    let t = line.trim();
    if !t.starts_with('{') {
        return None;
    }
    let v: Value = serde_json::from_str(t).ok()?;
    let stage = v.get("stage")?.as_str()?.to_string();
    let event = v.get("event")?.as_str()?.to_string();
    if !matches!(event.as_str(), "start" | "progress" | "done" | "error" | "log") {
        return None;
    }
    let pct = v.get("pct").and_then(|p| p.as_f64()).map(|p| p.round().clamp(0.0, 100.0) as u32);
    let message = v.get("message").and_then(|m| m.as_str()).unwrap_or("").to_string();
    let detail = v.get("detail").and_then(|d| d.as_str()).filter(|d| !d.is_empty()).map(|d| d.to_string());
    let at = v.get("at").and_then(|a| a.as_i64()).unwrap_or_else(now_ms);
    Some(ProgressLine { stage, event, pct, message, detail, at })
}

impl ProgressLine {
    pub fn to_event(&self, song_id: &str) -> ProgressEvent {
        ProgressEvent {
            song_id: song_id.to_string(),
            stage: self.stage.clone(),
            event: self.event.clone(),
            pct: self.pct,
            message: self.message.clone(),
            detail: self.detail.clone(),
            at: self.at,
        }
    }
}

fn line(stage: &str, event: &str, message: impl Into<String>) -> ProgressLine {
    ProgressLine { stage: stage.into(), event: event.into(), pct: None, message: message.into(), detail: None, at: now_ms() }
}

// ------------------------------------------------------------- bookkeeping

/// Runs one song's pipeline from `from` and keeps `song.json` current. The
/// caller is responsible for the one-at-a-time guard.
pub async fn run_pipeline(ctx: &Ctx, id: &str, from: &str, sink: &Sink, cancel: &Cancel) -> Result<Song, String> {
    if !RUN_STAGES.contains(&from) {
        return Err(format!("not a stage to start from: {from:?}"));
    }
    let settings = store::load_settings(&ctx.data_dir);
    let mut song = store::load_song(&ctx.data_dir, id)?;
    let song_dir = store::song_dir(&ctx.data_dir, id)?;
    if song.lyrics.trim().is_empty() {
        return Err("This song has no lyrics yet.".into());
    }
    store::save_lyrics_txt(&ctx.data_dir, &song)?;

    let convert_on = settings.convert && !settings.voice_model.pth.trim().is_empty();
    reset_from(&mut song, &song_dir, from, convert_on);
    if from == "render" {
        song.engine = if settings.engine.trim() == "fal" { "fal".into() } else { "local".into() };
        song.steps = settings.steps;
    }
    store::save_song(&ctx.data_dir, &song)?;

    let mut run = Run { ctx, song, song_dir, sink, cancel, render_started: None, error_reported: false };
    let result = run.go(from, &settings, convert_on).await;

    run.refresh_files();
    if let Err(msg) = &result {
        // Whatever failed, the song remembers why. If the engine already
        // named the broken stage, leave it; otherwise it is the one that was
        // running, or the first that never got going. The UI always gets an
        // error event, even when the failure came before the engine started.
        let already = RUN_STAGES.iter().copied().find(|s| run.song.stages.get(s) == Some(ERROR));
        let stage = already.or_else(|| run.song.stages.running()).or_else(|| RUN_STAGES.iter().copied().find(|s| run.song.stages.get(s) == Some(PENDING)));
        if let Some(stage) = stage {
            run.song.stages.set(stage, ERROR);
            if !run.error_reported {
                (run.sink)(line(stage, "error", msg.clone()).to_event(&run.song.id));
            }
        }
        // The engine's own message is the readable one; the returned error
        // carries the traceback tail as well, which belongs in engine.log.
        if run.song.error.is_none() {
            run.song.error = Some(msg.clone());
        }
    }
    let _ = store::save_song(&ctx.data_dir, &run.song);
    result.map(|_| run.song)
}

/// Stages from `from` onward go back to pending (convert: skipped when off),
/// their old outputs are removed so a failed re-run cannot show stale files
/// as fresh, and the last error is cleared.
fn reset_from(song: &mut Song, song_dir: &Path, from: &str, convert_on: bool) {
    let start = RUN_STAGES.iter().position(|s| *s == from).unwrap_or(0);
    for stage in &RUN_STAGES[start..] {
        let status = if *stage == "convert" && !convert_on { SKIPPED } else { PENDING };
        song.stages.set(stage, status);
        if let Some((_, files)) = OUTPUTS.iter().find(|(s, _)| s == stage) {
            for f in *files {
                let _ = std::fs::remove_file(song_dir.join(f));
            }
        }
    }
    song.error = None;
    if start == 0 {
        song.render_seconds = None;
        song.actual_duration_sec = None;
    }
}

struct Run<'a> {
    ctx: &'a Ctx,
    song: Song,
    song_dir: PathBuf,
    sink: &'a Sink,
    cancel: &'a Cancel,
    render_started: Option<Instant>,
    /// An error event has gone to the UI already; don't send a second one.
    error_reported: bool,
}

impl Run<'_> {
    async fn go(&mut self, from: &str, settings: &Settings, convert_on: bool) -> Result<(), String> {
        let mut from = from;
        if from == "render" && settings.engine.trim() == "fal" {
            self.fal_render(settings).await?;
            from = "separate";
        }
        self.run_engine(from, settings, convert_on).await
    }

    /// Forward a protocol line to the UI and update the song.
    fn handle(&mut self, p: &ProgressLine) {
        (self.sink)(p.to_event(&self.song.id));
        let known = RUN_STAGES.contains(&p.stage.as_str());
        match p.event.as_str() {
            "start" if known => {
                self.song.stages.set(&p.stage, RUNNING);
                if p.stage == "render" {
                    self.render_started = Some(Instant::now());
                }
            }
            "done" if known => {
                self.song.stages.set(&p.stage, DONE);
                if p.stage == "render" {
                    self.song.render_seconds = self.render_started.map(|t| (t.elapsed().as_secs_f64() * 10.0).round() / 10.0);
                    self.song.actual_duration_sec = wav_duration(&self.song_dir.join("raw.wav"));
                }
                self.refresh_files();
            }
            "error" => {
                let stage = if known { p.stage.as_str() } else { self.song.stages.running().unwrap_or("render") };
                self.song.stages.set(stage, ERROR);
                self.song.error = Some(p.message.clone());
                self.error_reported = true;
            }
            _ => return,
        }
        let _ = store::save_song(&self.ctx.data_dir, &self.song);
    }

    /// `files` lists what is actually in the folder, nothing more.
    fn refresh_files(&mut self) {
        let has = |name: &str| self.song_dir.join(name).is_file().then(|| name.to_string());
        let f = &mut self.song.files;
        f.raw = has("raw.wav");
        f.vocals = has("vocals.wav");
        f.instrumental = has("instrumental.wav");
        f.converted = has("vocals_converted.wav");
        f.final_ = has("final.wav");
        f.mp3 = has("final.mp3");
        f.cover = has("cover.png");
    }

    fn log(&self, text: &str) {
        append_log(&self.song_dir.join("engine.log"), text);
    }

    // ---------------------------------------------------------- local engine

    async fn run_engine(&mut self, from: &str, settings: &Settings, convert_on: bool) -> Result<(), String> {
        let python = venv_python(&self.ctx.data_dir);
        if !python.is_file() {
            return Err("The engine isn't installed yet. Open Setup and press Install engine.".into());
        }
        let args = run_args(&self.song_dir, from, settings, convert_on);
        self.log(&format!("\n=== {} run --from {from}\n$ {} -m overnight_engine {}\n", chrono::Local::now().format("%Y-%m-%d %H:%M:%S"), python.display(), args.join(" ")));

        let mut cmd = tokio::process::Command::new(&python);
        cmd.args(["-m", "overnight_engine"]).args(&args);
        self.engine_env(&mut cmd, settings);
        cmd.current_dir(store::songs_dir(&self.ctx.data_dir))
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        let mut child = cmd.spawn().map_err(|e| format!("Couldn't start the engine ({}): {e}", python.display()))?;

        // stderr straight to engine.log as it comes, so a chatty library
        // cannot fill the pipe and stall the run.
        let stderr = child.stderr.take().ok_or("no stderr from the engine")?;
        let log_path = self.song_dir.join("engine.log");
        let stderr_task = tokio::spawn(async move {
            let mut lines = BufReader::new(stderr).lines();
            let mut tail: Vec<String> = Vec::new();
            while let Ok(Some(l)) = lines.next_line().await {
                append_log(&log_path, &format!("{l}\n"));
                if !l.trim().is_empty() {
                    tail.push(l);
                    if tail.len() > 8 {
                        tail.remove(0);
                    }
                }
            }
            tail.join("\n")
        });

        let stdout = child.stdout.take().ok_or("no stdout from the engine")?;
        let mut lines = BufReader::new(stdout).lines();
        let mut error_seen: Option<String> = None;
        loop {
            let next = tokio::select! {
                l = lines.next_line() => l,
                _ = self.cancel.wait() => {
                    let _ = child.kill().await;
                    let stage = self.song.stages.running().unwrap_or(from);
                    self.handle(&line(stage, "error", "Cancelled"));
                    return Err("Cancelled".into());
                }
            };
            let l = match next {
                Ok(Some(l)) => l,
                Ok(None) => break,
                Err(e) => return Err(format!("Lost the connection to the engine: {e}")),
            };
            match parse_progress_line(&l) {
                Some(p) => {
                    if p.event == "error" {
                        error_seen = Some(match &p.detail {
                            Some(d) => format!("{}\n{d}", p.message),
                            None => p.message.clone(),
                        });
                    }
                    self.handle(&p);
                }
                None => self.log(&format!("{l}\n")),
            }
        }

        let status = child.wait().await.map_err(|e| format!("engine: {e}"))?;
        let stderr_tail = tokio::time::timeout(Duration::from_secs(2), stderr_task).await.ok().and_then(|r| r.ok()).unwrap_or_default();
        if let Some(msg) = error_seen {
            return Err(msg);
        }
        if !status.success() {
            let code = status.code().map(|c| c.to_string()).unwrap_or_else(|| "signal".into());
            let mut msg = format!("The engine stopped (exit code {code}).");
            if !stderr_tail.trim().is_empty() {
                msg.push_str(&format!(" {}", crate::lyrics::truncate(stderr_tail.trim(), 400)));
            }
            return Err(msg);
        }
        // A clean exit with a stage never announced: the engine had nothing
        // to do for it (resumed past it, or skipped convert). Say so.
        let start = RUN_STAGES.iter().position(|s| *s == from).unwrap_or(0);
        for stage in &RUN_STAGES[start..] {
            if self.song.stages.get(stage) == Some(PENDING) || self.song.stages.get(stage) == Some(RUNNING) {
                let produced = OUTPUTS.iter().find(|(s, _)| s == stage).map(|(_, f)| f.iter().all(|n| self.song_dir.join(n).is_file())).unwrap_or(false);
                self.song.stages.set(stage, if produced { DONE } else { SKIPPED });
            }
        }
        Ok(())
    }

    /// The env of SPEC §3: where the data and the weights are, which device,
    /// and the switches that keep torch and Python from misbehaving.
    fn engine_env(&self, cmd: &mut tokio::process::Command, settings: &Settings) {
        let models = store::models_dir(&self.ctx.data_dir);
        cmd.env("PATH", &self.ctx.path)
            .env("OVERNIGHT_DATA", &self.ctx.data_dir)
            .env("OVERNIGHT_MODELS", &models)
            .env("HF_HOME", &models)
            .env("TORCH_HOME", &models)
            .env("ACE_STEP_CHECKPOINTS", models.join("acestep"))
            .env("OVERNIGHT_DEVICE", resolve_device(&settings.device))
            .env("PYTORCH_ENABLE_MPS_FALLBACK", "1")
            .env("PYTHONUNBUFFERED", "1")
            // The package is run from source, not installed into the venv.
            .env("PYTHONPATH", &self.ctx.engine_dir);
    }

    // ------------------------------------------------------------------ fal

    /// Render on fal.ai instead of locally: submit, poll, download raw.wav.
    async fn fal_render(&mut self, settings: &Settings) -> Result<(), String> {
        self.handle(&line("render", "start", "Sending to fal.ai"));
        let key = settings.fal_key.trim().to_string();
        let result = if key.is_empty() {
            Err("The fal engine needs a fal.ai key. Add it in Setup.".to_string())
        } else {
            self.fal_render_inner(&key, settings).await
        };
        match &result {
            Ok(()) => {
                let secs = self.song.actual_duration_sec.or_else(|| wav_duration(&self.song_dir.join("raw.wav")));
                let msg = match secs {
                    Some(s) => format!("raw.wav {s:.1} s"),
                    None => "raw.wav".to_string(),
                };
                self.handle(&line("render", "done", msg));
            }
            Err(e) => self.handle(&line("render", "error", e.clone())),
        }
        result
    }

    async fn fal_render_inner(&mut self, key: &str, settings: &Settings) -> Result<(), String> {
        let client = reqwest::Client::builder().timeout(Duration::from_secs(120)).build().map_err(|e| e.to_string())?;
        let auth = format!("Key {key}");
        let body = fal_request_body(&self.song, settings);
        let submitted: Value = client
            .post(FAL_SUBMIT_URL)
            .header("Authorization", &auth)
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("fal.ai: {e}"))?
            .error_for_status()
            .map_err(|e| format!("fal.ai refused the request: {e}"))?
            .json()
            .await
            .map_err(|e| format!("fal.ai reply: {e}"))?;
        let req = parse_fal_submit(&submitted)?;
        self.log(&format!("fal request {}\n", req.request_id));

        let deadline = Instant::now() + Duration::from_secs(15 * 60);
        let mut last_msg = String::new();
        loop {
            if self.cancel.is_cancelled() {
                return Err("Cancelled".into());
            }
            if Instant::now() > deadline {
                return Err("fal.ai took more than 15 minutes.".into());
            }
            let status: Value = client
                .get(&req.status_url)
                .query(&[("logs", "1")])
                .header("Authorization", &auth)
                .send()
                .await
                .map_err(|e| format!("fal.ai status: {e}"))?
                .json()
                .await
                .map_err(|e| format!("fal.ai status: {e}"))?;
            let st = parse_fal_status(&status);
            for l in &st.logs {
                self.log(&format!("fal: {l}\n"));
            }
            let msg = match st.status.as_str() {
                "IN_QUEUE" => match st.queue_position {
                    Some(n) => format!("Queued on fal.ai (position {n})"),
                    None => "Queued on fal.ai".into(),
                },
                "IN_PROGRESS" => st.logs.last().cloned().unwrap_or_else(|| "Rendering on fal.ai".into()),
                "COMPLETED" => break,
                other => return Err(format!("fal.ai reported {other}")),
            };
            if msg != last_msg {
                self.handle(&ProgressLine { pct: None, ..line("render", "progress", msg.clone()) });
                last_msg = msg;
            }
            tokio::select! {
                _ = tokio::time::sleep(Duration::from_secs(2)) => {}
                _ = self.cancel.wait() => return Err("Cancelled".into()),
            }
        }

        let result: Value = client
            .get(&req.response_url)
            .header("Authorization", &auth)
            .send()
            .await
            .map_err(|e| format!("fal.ai result: {e}"))?
            .json()
            .await
            .map_err(|e| format!("fal.ai result: {e}"))?;
        let url = parse_fal_audio_url(&result)?;
        self.handle(&ProgressLine { pct: Some(95), ..line("render", "progress", "Downloading") });
        download(&client, &url, &self.song_dir.join("raw.wav")).await?;
        self.song.actual_duration_sec = wav_duration(&self.song_dir.join("raw.wav"));
        Ok(())
    }
}

/// Arguments for `python -m overnight_engine run ...` (SPEC §3.1).
pub fn run_args(song_dir: &Path, from: &str, settings: &Settings, convert_on: bool) -> Vec<String> {
    let mut a: Vec<String> = vec!["run".into(), "--dir".into(), song_dir.display().to_string(), "--from".into(), from.into()];
    a.extend(["--guidance".to_string(), fmt_f(settings.guidance)]);
    if from == "render" {
        // Starting from render is an explicit ask for a fresh take, not a resume.
        a.push("--force".into());
    }
    if convert_on {
        let vm = &settings.voice_model;
        a.extend(["--pth".to_string(), vm.pth.trim().to_string()]);
        if !vm.index.trim().is_empty() {
            a.extend(["--index".to_string(), vm.index.trim().to_string()]);
        }
        a.extend(["--pitch".to_string(), vm.pitch.to_string()]);
        a.extend(["--method".to_string(), vm.method.trim().to_string()]);
        a.extend(["--index-rate".to_string(), fmt_f(vm.index_rate)]);
        a.extend(["--protect".to_string(), fmt_f(vm.protect)]);
        a.extend(["--filter-radius".to_string(), vm.filter_radius.to_string()]);
        a.extend(["--rms-mix-rate".to_string(), fmt_f(vm.rms_mix_rate)]);
    } else {
        a.push("--no-convert".into());
    }
    a.extend(["--vocals-gain-db".to_string(), fmt_f(settings.vocals_gain_db)]);
    a.extend(["--bitrate".to_string(), settings.mp3_bitrate.to_string()]);
    a
}

fn fmt_f(f: f64) -> String {
    // "15" rather than "15.0" reads better on a command line; both parse.
    if f.fract() == 0.0 {
        format!("{}", f as i64)
    } else {
        format!("{f}")
    }
}

fn append_log(path: &Path, text: &str) {
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        let _ = f.write_all(text.as_bytes());
    }
}

// --------------------------------------------------------------------- fal

pub const FAL_SUBMIT_URL: &str = "https://queue.fal.run/fal-ai/ace-step";

#[derive(Debug, Clone, PartialEq)]
pub struct FalRequest {
    pub request_id: String,
    pub status_url: String,
    pub response_url: String,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct FalStatus {
    /// IN_QUEUE | IN_PROGRESS | COMPLETED (or whatever else fal says)
    pub status: String,
    pub queue_position: Option<u32>,
    pub logs: Vec<String>,
}

pub fn fal_request_body(song: &Song, settings: &Settings) -> Value {
    json!({
        "tags": song.tags,
        "lyrics": song.lyrics,
        "duration": song.duration_sec,
        "number_of_steps": settings.steps,
        "guidance_scale": settings.guidance,
        "seed": song.seed,
    })
}

/// The queue's answer to a submit. The URLs it gives are preferred; the
/// documented shapes are the fallback.
pub fn parse_fal_submit(v: &Value) -> Result<FalRequest, String> {
    let id = v
        .get("request_id")
        .and_then(|r| r.as_str())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| format!("fal.ai didn't return a request id: {}", crate::lyrics::truncate(&v.to_string(), 200)))?;
    let s = |k: &str, fallback: String| v.get(k).and_then(|x| x.as_str()).filter(|x| x.starts_with("http")).map(|x| x.to_string()).unwrap_or(fallback);
    Ok(FalRequest {
        request_id: id.to_string(),
        status_url: s("status_url", format!("{FAL_SUBMIT_URL}/requests/{id}/status")),
        response_url: s("response_url", format!("{FAL_SUBMIT_URL}/requests/{id}")),
    })
}

pub fn parse_fal_status(v: &Value) -> FalStatus {
    let logs = v
        .get("logs")
        .and_then(|l| l.as_array())
        .map(|a| a.iter().filter_map(|l| l.get("message").and_then(|m| m.as_str()).or_else(|| l.as_str())).map(|m| m.trim().to_string()).filter(|m| !m.is_empty()).collect())
        .unwrap_or_default();
    FalStatus {
        status: v.get("status").and_then(|s| s.as_str()).unwrap_or("").to_uppercase(),
        queue_position: v.get("queue_position").and_then(|q| q.as_u64()).map(|q| q as u32),
        logs,
    }
}

/// `audio.url` of the finished request. Some models return `audio_file` or
/// a list; take the first thing that looks like an audio URL.
pub fn parse_fal_audio_url(v: &Value) -> Result<String, String> {
    let candidates = [v.pointer("/audio/url"), v.pointer("/audio_file/url"), v.pointer("/audio/0/url"), v.pointer("/output/audio/url"), v.get("url")];
    candidates
        .iter()
        .flatten()
        .find_map(|u| u.as_str())
        .filter(|u| u.starts_with("http"))
        .map(|u| u.to_string())
        .ok_or_else(|| format!("fal.ai finished but returned no audio url: {}", crate::lyrics::truncate(&v.to_string(), 200)))
}

/// Stream to a `.part` file and rename, so a half download never looks like raw.wav.
async fn download(client: &reqwest::Client, url: &str, dest: &Path) -> Result<(), String> {
    use futures::StreamExt as _;
    let part = dest.with_extension("wav.part");
    let mut file = std::fs::File::create(&part).map_err(|e| format!("write {}: {e}", part.display()))?;
    let resp = client.get(url).send().await.map_err(|e| format!("download: {e}"))?.error_for_status().map_err(|e| format!("download: {e}"))?;
    let mut stream = resp.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| format!("download: {e}"))?;
        file.write_all(&chunk).map_err(|e| format!("write {}: {e}", part.display()))?;
    }
    drop(file);
    std::fs::rename(&part, dest).map_err(|e| format!("rename {}: {e}", dest.display()))
}

// --------------------------------------------------------------------- wav

/// Seconds of audio in a PCM WAV, from its header alone (no decoding).
pub fn wav_duration(path: &Path) -> Option<f64> {
    let mut f = std::fs::File::open(path).ok()?;
    let mut head = [0u8; 12];
    f.read_exact(&mut head).ok()?;
    if &head[0..4] != b"RIFF" || &head[8..12] != b"WAVE" {
        return None;
    }
    let mut byte_rate: Option<u32> = None;
    loop {
        let mut ch = [0u8; 8];
        if f.read_exact(&mut ch).is_err() {
            return None;
        }
        let size = u32::from_le_bytes([ch[4], ch[5], ch[6], ch[7]]);
        match &ch[0..4] {
            b"fmt " => {
                let mut fmt = vec![0u8; size as usize];
                f.read_exact(&mut fmt).ok()?;
                if fmt.len() >= 12 {
                    byte_rate = Some(u32::from_le_bytes([fmt[8], fmt[9], fmt[10], fmt[11]]));
                }
            }
            b"data" => {
                let rate = byte_rate.filter(|r| *r > 0)? as f64;
                // A streaming writer may leave the size unset; fall back to the file length.
                let bytes = if size == 0 || size == u32::MAX {
                    let here = f.stream_position().ok()?;
                    f.metadata().ok()?.len().saturating_sub(here)
                } else {
                    size as u64
                };
                return Some(((bytes as f64 / rate) * 10.0).round() / 10.0);
            }
            _ => {
                // Chunks are word aligned.
                f.seek(SeekFrom::Current(size as i64 + (size % 2) as i64)).ok()?;
            }
        }
    }
}

// ------------------------------------------------------------- check/setup

/// The engine's own report (SPEC §3.4). None when there is no venv or the
/// engine could not even start.
pub async fn check(ctx: &Ctx, settings: &Settings) -> Option<Value> {
    let python = venv_python(&ctx.data_dir);
    if !python.is_file() {
        return None;
    }
    let models = store::models_dir(&ctx.data_dir);
    let mut cmd = tokio::process::Command::new(&python);
    cmd.args(["-m", "overnight_engine", "check"])
        .env("PATH", &ctx.path)
        .env("OVERNIGHT_DATA", &ctx.data_dir)
        .env("OVERNIGHT_MODELS", &models)
        .env("HF_HOME", &models)
        .env("TORCH_HOME", &models)
        .env("ACE_STEP_CHECKPOINTS", models.join("acestep"))
        .env("OVERNIGHT_DEVICE", resolve_device(&settings.device))
        .env("PYTORCH_ENABLE_MPS_FALLBACK", "1")
        .env("PYTHONUNBUFFERED", "1")
        .env("PYTHONPATH", &ctx.engine_dir)
        .current_dir(&ctx.data_dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let out = tokio::time::timeout(Duration::from_secs(90), cmd.output()).await.ok()?.ok()?;
    let stdout = String::from_utf8_lossy(&out.stdout);
    match crate::lyrics::extract_json(&stdout) {
        Some(v) => Some(v),
        None => {
            let err = String::from_utf8_lossy(&out.stderr);
            Some(json!({ "problems": [format!("engine check failed: {}", crate::lyrics::truncate(err.trim(), 300))] }))
        }
    }
}

/// Run `setup.sh <venv> <models>` and hand every line of its output to `on_line`.
pub async fn install(ctx: &Ctx, uv_bin: Option<&Path>, on_line: &(dyn Fn(String) + Send + Sync)) -> Result<(), String> {
    let script = ctx.engine_dir.join("setup.sh");
    if !script.is_file() {
        return Err(format!("No setup.sh in {}", ctx.engine_dir.display()));
    }
    let venv = store::venv_dir(&ctx.data_dir);
    let models = store::models_dir(&ctx.data_dir);
    // uv may live somewhere the login shell does not list; put its folder first.
    let mut path = ctx.path.clone();
    if let Some(dir) = uv_bin.and_then(|p| p.parent()) {
        path = format!("{}:{path}", dir.display());
    }
    let log_path = ctx.data_dir.join("engine.log");
    append_log(&log_path, &format!("\n=== {} setup.sh {} {}\n", chrono::Local::now().format("%Y-%m-%d %H:%M:%S"), venv.display(), models.display()));

    let mut cmd = tokio::process::Command::new("/bin/bash");
    cmd.arg(&script)
        .arg(&venv)
        .arg(&models)
        .env("PATH", &path)
        .env("OVERNIGHT_DATA", &ctx.data_dir)
        .env("OVERNIGHT_MODELS", &models)
        .env("HF_HOME", &models)
        .env("TORCH_HOME", &models)
        .env("PYTHONUNBUFFERED", "1")
        .current_dir(&ctx.engine_dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let mut child = cmd.spawn().map_err(|e| format!("Couldn't start setup.sh: {e}"))?;
    let stdout = child.stdout.take().ok_or("no stdout from setup.sh")?;
    let stderr = child.stderr.take().ok_or("no stderr from setup.sh")?;

    // Both streams into one channel, in arrival order.
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    fn pump<R: tokio::io::AsyncRead + Unpin + Send + 'static>(reader: R, tx: tokio::sync::mpsc::UnboundedSender<String>) {
        tokio::spawn(async move {
            let mut lines = tokio::io::BufReader::new(reader).lines();
            while let Ok(Some(l)) = lines.next_line().await {
                if tx.send(l).is_err() {
                    break;
                }
            }
        });
    }
    pump(stdout, tx.clone());
    pump(stderr, tx);
    let mut tail: Vec<String> = Vec::new();
    while let Some(l) = rx.recv().await {
        append_log(&log_path, &format!("{l}\n"));
        // The script speaks the progress protocol too; show the message, not the JSON.
        let shown = parse_progress_line(&l).map(|p| p.message).unwrap_or(l);
        if !shown.trim().is_empty() {
            tail.push(shown.clone());
            if tail.len() > 6 {
                tail.remove(0);
            }
            on_line(shown);
        }
    }
    let status = child.wait().await.map_err(|e| format!("setup.sh: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("setup.sh failed (exit code {}). {}", status.code().unwrap_or(-1), tail.join(" | ")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_protocol_lines_and_ignores_noise() {
        let p = parse_progress_line(r#"{"stage":"render","event":"progress","pct":42.4,"message":"Step 25/60","at":1727650000123}"#).unwrap();
        assert_eq!(p.stage, "render");
        assert_eq!(p.event, "progress");
        assert_eq!(p.pct, Some(42));
        assert_eq!(p.message, "Step 25/60");
        assert_eq!(p.at, 1727650000123);
        let e = parse_progress_line(r#"{"stage":"separate","event":"error","message":"boom","detail":"Traceback..."}"#).unwrap();
        assert_eq!(e.detail.as_deref(), Some("Traceback..."));
        assert!(e.at > 0, "missing at is filled in");
        assert!(parse_progress_line("Loading checkpoint shards: 100%").is_none());
        assert!(parse_progress_line(r#"{"foo":1}"#).is_none());
        assert!(parse_progress_line(r#"{"stage":"render","event":"party"}"#).is_none());
        assert_eq!(parse_progress_line(r#"{"stage":"mix","event":"progress","pct":250}"#).unwrap().pct, Some(100));
        let ev = p.to_event("20260929-153000-k3q9");
        assert_eq!(ev.song_id, "20260929-153000-k3q9");
        assert_eq!(ev.pct, Some(42));
    }

    #[test]
    fn fal_submit_and_status_and_result_parse() {
        let submit: Value = serde_json::json!({
            "request_id": "abc-123",
            "response_url": "https://queue.fal.run/fal-ai/ace-step/requests/abc-123",
            "status_url": "https://queue.fal.run/fal-ai/ace-step/requests/abc-123/status",
            "cancel_url": "https://queue.fal.run/fal-ai/ace-step/requests/abc-123/cancel"
        });
        let req = parse_fal_submit(&submit).unwrap();
        assert_eq!(req.request_id, "abc-123");
        assert!(req.status_url.ends_with("/abc-123/status"));
        // Only the id: the documented URLs are built.
        let bare = parse_fal_submit(&serde_json::json!({"request_id": "x1"})).unwrap();
        assert_eq!(bare.status_url, "https://queue.fal.run/fal-ai/ace-step/requests/x1/status");
        assert_eq!(bare.response_url, "https://queue.fal.run/fal-ai/ace-step/requests/x1");
        assert!(parse_fal_submit(&serde_json::json!({"detail": "Unauthorized"})).is_err());

        let queued = parse_fal_status(&serde_json::json!({"status": "IN_QUEUE", "queue_position": 3}));
        assert_eq!(queued.status, "IN_QUEUE");
        assert_eq!(queued.queue_position, Some(3));
        let running = parse_fal_status(&serde_json::json!({"status": "IN_PROGRESS", "logs": [{"message": "loading", "level": "INFO"}, {"message": " step 3 "}]}));
        assert_eq!(running.logs, vec!["loading", "step 3"]);
        assert_eq!(parse_fal_status(&serde_json::json!({})).status, "");

        let result: Value = serde_json::json!({
            "audio": {"url": "https://v3.fal.media/files/abc/out.wav", "content_type": "audio/wav", "file_name": "out.wav", "file_size": 1234},
            "seed": 1234567,
            "tags": "x", "lyrics": "y"
        });
        assert_eq!(parse_fal_audio_url(&result).unwrap(), "https://v3.fal.media/files/abc/out.wav");
        assert!(parse_fal_audio_url(&serde_json::json!({"audio": {}})).is_err());
    }

    #[test]
    fn fal_body_matches_the_spec_fields() {
        let song = Song { tags: "t".into(), lyrics: "l".into(), duration_sec: 150, seed: 7, ..Default::default() };
        let b = fal_request_body(&song, &Settings::default());
        assert_eq!(b["tags"], "t");
        assert_eq!(b["lyrics"], "l");
        assert_eq!(b["duration"], 150);
        assert_eq!(b["number_of_steps"], 60);
        assert_eq!(b["guidance_scale"], 15.0);
        assert_eq!(b["seed"], 7);
    }

    #[test]
    fn run_args_follow_the_settings() {
        let mut s = Settings::default();
        let dir = Path::new("/tmp/songs/x");
        let a = run_args(dir, "render", &s, false).join(" ");
        assert!(a.starts_with("run --dir /tmp/songs/x --from render --guidance 15 --force"));
        assert!(a.contains("--no-convert"));
        assert!(a.ends_with("--vocals-gain-db 0 --bitrate 320"));
        s.convert = true;
        s.voice_model.pth = "/v/m.pth".into();
        s.voice_model.pitch = -2;
        let a = run_args(dir, "mix", &s, true).join(" ");
        assert!(!a.contains("--force"));
        assert!(a.contains("--pth /v/m.pth --pitch -2 --method rmvpe --index-rate 0.66 --protect 0.33 --filter-radius 3 --rms-mix-rate 0.25"));
        assert!(!a.contains("--index "));
        assert!(!a.contains("--no-convert"));
    }

    #[test]
    fn reads_a_wav_header() {
        // 1 s of 16-bit mono at 8 kHz, plus a LIST chunk before data to skip.
        let mut w: Vec<u8> = Vec::new();
        w.extend(b"RIFF");
        w.extend(0u32.to_le_bytes());
        w.extend(b"WAVE");
        w.extend(b"fmt ");
        w.extend(16u32.to_le_bytes());
        w.extend(1u16.to_le_bytes()); // pcm
        w.extend(1u16.to_le_bytes()); // mono
        w.extend(8000u32.to_le_bytes());
        w.extend(16000u32.to_le_bytes()); // byte rate
        w.extend(2u16.to_le_bytes());
        w.extend(16u16.to_le_bytes());
        w.extend(b"LIST");
        w.extend(3u32.to_le_bytes());
        w.extend([1u8, 2, 3, 0]); // odd chunk, padded
        w.extend(b"data");
        w.extend(16000u32.to_le_bytes());
        w.extend(vec![0u8; 16000]);
        let p = std::env::temp_dir().join(format!("overnight-wav-{}.wav", std::process::id()));
        std::fs::write(&p, &w).unwrap();
        assert_eq!(wav_duration(&p), Some(1.0));
        std::fs::write(&p, b"not a wav").unwrap();
        assert_eq!(wav_duration(&p), None);
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn device_resolves() {
        assert_eq!(resolve_device("cpu"), "cpu");
        assert_eq!(resolve_device("mps"), "mps");
        assert!(matches!(resolve_device("auto"), "mps" | "cpu"));
    }

    #[test]
    fn locate_engine_prefers_the_bundled_resource() {
        let dir = std::env::temp_dir().join(format!("overnight-engine-{}", std::process::id()));
        let res_dir = dir.join("res");
        std::fs::create_dir_all(res_dir.join("resources/engine/overnight_engine")).unwrap();
        assert_eq!(locate_engine(Some(&res_dir)).unwrap(), res_dir.join("resources/engine"));
        // An empty resource dir: either the dev fallback finds the repo's
        // engine/ or the error says where it looked.
        match locate_engine(Some(&dir.join("empty"))) {
            Ok(p) => assert!(is_engine_dir(&p)),
            Err(e) => assert!(e.contains("Looked in") && e.contains("empty/resources/engine"), "{e}"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn cancel_wakes_waiters() {
        let c = Cancel::new();
        let c2 = c.clone();
        let waiter = tokio::spawn(async move { c2.wait().await });
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert!(!waiter.is_finished());
        c.cancel();
        tokio::time::timeout(Duration::from_secs(1), waiter).await.expect("woke").unwrap();
        assert!(c.is_cancelled());
        // Cancelling before waiting returns at once.
        tokio::time::timeout(Duration::from_millis(100), c.wait()).await.expect("immediate");
    }
}
