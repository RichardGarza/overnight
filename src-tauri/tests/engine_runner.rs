//! Drives `engine::run_pipeline` against tests/fake_engine.py in a scratch
//! data folder: the protocol parsing, song.json bookkeeping, engine.log,
//! failure, cancel and the convert switch, with no models and no venv.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use overnight_lib::engine::{self, Cancel, Ctx};
use overnight_lib::model::{ProgressEvent, Settings, Song, DONE, ERROR, PENDING, SKIPPED};
use overnight_lib::store;

struct Scratch {
    ctx: Ctx,
    events: Arc<Mutex<Vec<ProgressEvent>>>,
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.ctx.data_dir);
        let _ = std::fs::remove_dir_all(&self.ctx.engine_dir);
    }
}

fn python3() -> PathBuf {
    let path = std::env::var("PATH").unwrap_or_default();
    path.split(':')
        .map(|d| Path::new(d).join("python3"))
        .chain(["/opt/homebrew/bin/python3", "/usr/bin/python3"].iter().map(PathBuf::from))
        .find(|p| p.is_file())
        .expect("a python3 to run the fake engine with")
}

/// A data dir whose "venv" is a symlink to the system python, and an engine
/// dir whose `overnight_engine` package just runs fake_engine.py.
fn scratch(name: &str) -> Scratch {
    let base = std::env::temp_dir().join(format!("overnight-it-{name}-{}-{}", std::process::id(), chrono_ms()));
    let data_dir = base.join("data");
    let engine_dir = base.join("engine");
    store::ensure_layout(&data_dir).unwrap();
    std::fs::create_dir_all(store::venv_dir(&data_dir).join("bin")).unwrap();
    std::os::unix::fs::symlink(python3(), engine::venv_python(&data_dir)).unwrap();
    let pkg = engine_dir.join("overnight_engine");
    std::fs::create_dir_all(&pkg).unwrap();
    std::fs::write(pkg.join("__init__.py"), "").unwrap();
    let fake = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fake_engine.py");
    std::fs::write(pkg.join("__main__.py"), format!("import runpy\nrunpy.run_path({:?}, run_name=\"__main__\")\n", fake.display().to_string())).unwrap();
    let path = std::env::var("PATH").unwrap_or_default();
    Scratch { ctx: Ctx { data_dir, engine_dir, path }, events: Arc::new(Mutex::new(Vec::new())) }
}

fn chrono_ms() -> i64 {
    overnight_lib::model::now_ms()
}

fn song(dir: &Path) -> Song {
    let s = Song {
        id: store::new_song_id(),
        title: "Left On Read".into(),
        theme: "she never answered".into(),
        lyrics: "[verse]\nline one\nline two\n\n[chorus]\nhook".into(),
        tags: "melodic rap, moody, 82 bpm".into(),
        bpm: 82,
        duration_sec: 30,
        seed: 42,
        steps: 27,
        ..Default::default()
    };
    store::save_song(dir, &s).unwrap();
    s
}

fn sink(events: &Arc<Mutex<Vec<ProgressEvent>>>) -> impl Fn(ProgressEvent) + Send + Sync {
    let events = events.clone();
    move |e| events.lock().unwrap().push(e)
}

#[tokio::test]
async fn full_run_marks_every_stage_and_file() {
    let sc = scratch("full");
    let s = song(&sc.ctx.data_dir);
    let song_dir = store::song_dir(&sc.ctx.data_dir, &s.id).unwrap();
    let t0 = std::time::Instant::now();
    let out = engine::run_pipeline(&sc.ctx, &s.id, "render", &sink(&sc.events), &Cancel::new()).await.expect("pipeline ok");
    println!("fake full run took {:.2}s", t0.elapsed().as_secs_f64());

    assert_eq!(out.stages.render, DONE);
    assert_eq!(out.stages.separate, DONE);
    assert_eq!(out.stages.convert, SKIPPED, "convert is off by default");
    assert_eq!(out.stages.mix, DONE);
    assert_eq!(out.error, None);
    assert_eq!(out.engine, "local");
    assert_eq!(out.files.raw.as_deref(), Some("raw.wav"));
    assert_eq!(out.files.vocals.as_deref(), Some("vocals.wav"));
    assert_eq!(out.files.instrumental.as_deref(), Some("instrumental.wav"));
    assert_eq!(out.files.converted, None);
    assert_eq!(out.files.final_.as_deref(), Some("final.wav"));
    assert_eq!(out.files.mp3.as_deref(), Some("final.mp3"));
    assert_eq!(out.files.cover.as_deref(), Some("cover.png"));
    assert_eq!(out.actual_duration_sec, Some(1.0), "read from the fake raw.wav header");
    assert!(out.render_seconds.is_some());

    // What is on disk agrees with what came back.
    let on_disk = store::load_song(&sc.ctx.data_dir, &s.id).unwrap();
    assert_eq!(on_disk, out);
    assert!(song_dir.join("lyrics.txt").is_file());
    let log = std::fs::read_to_string(song_dir.join("engine.log")).unwrap();
    assert!(log.contains("a library printed this to stdout by mistake"), "non-protocol stdout lands in engine.log");
    assert!(log.contains("stage mix finished"), "stderr lands in engine.log");
    assert!(log.contains("--from render"), "the command line is logged");

    let events = sc.events.lock().unwrap();
    let render: Vec<&ProgressEvent> = events.iter().filter(|e| e.stage == "render").collect();
    assert_eq!(render.first().map(|e| e.event.as_str()), Some("start"));
    assert_eq!(render.last().map(|e| e.event.as_str()), Some("done"));
    assert!(render.iter().any(|e| e.event == "progress" && e.pct == Some(100)));
    assert!(render.iter().any(|e| e.event == "log"));
    assert!(events.iter().all(|e| e.song_id == s.id));
    assert!(events.iter().all(|e| e.at > 0));
    assert!(!events.iter().any(|e| e.stage == "convert"));
}

#[tokio::test]
async fn a_failing_stage_is_recorded_and_stops_the_run() {
    let sc = scratch("fail");
    let s = song(&sc.ctx.data_dir);
    let song_dir = store::song_dir(&sc.ctx.data_dir, &s.id).unwrap();
    std::fs::write(song_dir.join("fake-fail"), "separate").unwrap();
    let err = engine::run_pipeline(&sc.ctx, &s.id, "render", &sink(&sc.events), &Cancel::new()).await.expect_err("separate fails");
    assert!(err.contains("fake separate failed on purpose"), "{err}");
    assert!(err.contains("Traceback"), "the detail rides along: {err}");

    let on_disk = store::load_song(&sc.ctx.data_dir, &s.id).unwrap();
    assert_eq!(on_disk.stages.render, DONE);
    assert_eq!(on_disk.stages.separate, ERROR);
    assert_eq!(on_disk.stages.mix, PENDING);
    assert_eq!(on_disk.error.as_deref(), Some("fake separate failed on purpose"));
    assert_eq!(on_disk.files.raw.as_deref(), Some("raw.wav"));
    assert_eq!(on_disk.files.vocals, None);
    let events = sc.events.lock().unwrap();
    assert_eq!(events.iter().filter(|e| e.event == "error").count(), 1, "the engine's own error event, and no second one from the runner");
    assert!(events.iter().any(|e| e.stage == "separate" && e.event == "error"));
}

#[tokio::test]
async fn cancel_kills_the_engine_and_says_so() {
    let sc = scratch("cancel");
    let s = song(&sc.ctx.data_dir);
    let song_dir = store::song_dir(&sc.ctx.data_dir, &s.id).unwrap();
    // Each progress step now takes 3 s, so the run is mid-render when we cancel.
    std::fs::write(song_dir.join("fake-sleep"), "3").unwrap();
    let cancel = Cancel::new();
    let ctx = sc.ctx.clone();
    let id = s.id.clone();
    let events = sc.events.clone();
    let c2 = cancel.clone();
    let task = tokio::spawn(async move {
        let sink = sink(&events);
        engine::run_pipeline(&ctx, &id, "render", &sink, &c2).await
    });
    tokio::time::sleep(Duration::from_millis(600)).await;
    let t0 = std::time::Instant::now();
    cancel.cancel();
    let result = tokio::time::timeout(Duration::from_secs(5), task).await.expect("cancel returns promptly").unwrap();
    println!("cancel took {:.2}s", t0.elapsed().as_secs_f64());
    assert_eq!(result.unwrap_err(), "Cancelled");
    assert!(t0.elapsed() < Duration::from_secs(2), "the child was killed, not waited for");

    let on_disk = store::load_song(&sc.ctx.data_dir, &s.id).unwrap();
    assert_eq!(on_disk.stages.render, ERROR);
    assert_eq!(on_disk.error.as_deref(), Some("Cancelled"));
    assert_eq!(on_disk.files.raw, None);
    let events = sc.events.lock().unwrap();
    assert!(events.iter().any(|e| e.stage == "render" && e.event == "error" && e.message == "Cancelled"));
}

#[tokio::test]
async fn convert_runs_when_a_voice_model_is_set_and_remix_keeps_the_render() {
    let sc = scratch("convert");
    let settings = Settings { convert: true, voice_model: overnight_lib::model::VoiceModel { pth: "/nowhere/voice.pth".into(), ..Default::default() }, ..Default::default() };
    store::save_settings(&sc.ctx.data_dir, &settings).unwrap();
    let s = song(&sc.ctx.data_dir);
    let song_dir = store::song_dir(&sc.ctx.data_dir, &s.id).unwrap();
    let out = engine::run_pipeline(&sc.ctx, &s.id, "render", &sink(&sc.events), &Cancel::new()).await.expect("pipeline ok");
    assert_eq!(out.stages.convert, DONE);
    assert_eq!(out.files.converted.as_deref(), Some("vocals_converted.wav"));
    let log = std::fs::read_to_string(song_dir.join("engine.log")).unwrap();
    assert!(log.contains("--pth /nowhere/voice.pth"));

    // Re-mix: only mix runs again, the render and stems stay.
    let raw_before = std::fs::metadata(song_dir.join("raw.wav")).unwrap().modified().unwrap();
    std::fs::remove_file(song_dir.join("final.mp3")).unwrap();
    sc.events.lock().unwrap().clear();
    let out = engine::run_pipeline(&sc.ctx, &s.id, "mix", &sink(&sc.events), &Cancel::new()).await.expect("remix ok");
    assert_eq!(out.stages.render, DONE);
    assert_eq!(out.stages.convert, DONE);
    assert_eq!(out.stages.mix, DONE);
    assert_eq!(out.files.mp3.as_deref(), Some("final.mp3"));
    assert_eq!(std::fs::metadata(song_dir.join("raw.wav")).unwrap().modified().unwrap(), raw_before);
    let events = sc.events.lock().unwrap();
    assert!(events.iter().all(|e| e.stage == "mix"), "{:?}", events.iter().map(|e| &e.stage).collect::<Vec<_>>());
}

#[tokio::test]
async fn fal_without_a_key_fails_cleanly_before_any_network() {
    let sc = scratch("fal");
    store::update_settings(&sc.ctx.data_dir, |s| s.engine = "fal".into()).unwrap();
    let s = song(&sc.ctx.data_dir);
    let err = engine::run_pipeline(&sc.ctx, &s.id, "render", &sink(&sc.events), &Cancel::new()).await.expect_err("no key");
    assert!(err.contains("fal.ai key"), "{err}");
    let on_disk = store::load_song(&sc.ctx.data_dir, &s.id).unwrap();
    assert_eq!(on_disk.engine, "fal");
    assert_eq!(on_disk.stages.render, ERROR);
    let events = sc.events.lock().unwrap();
    assert_eq!(events.iter().map(|e| e.event.as_str()).collect::<Vec<_>>(), vec!["start", "error"]);
}

#[tokio::test]
async fn bad_input_is_refused_up_front() {
    let sc = scratch("refuse");
    let s = song(&sc.ctx.data_dir);
    let err = engine::run_pipeline(&sc.ctx, &s.id, "lyrics", &sink(&sc.events), &Cancel::new()).await.unwrap_err();
    assert!(err.contains("not a stage"));
    let err = engine::run_pipeline(&sc.ctx, "../../etc", "render", &sink(&sc.events), &Cancel::new()).await.unwrap_err();
    assert!(err.contains("not a song id"));
    // No venv: a message that points at Setup, and the song is marked.
    std::fs::remove_file(engine::venv_python(&sc.ctx.data_dir)).unwrap();
    let err = engine::run_pipeline(&sc.ctx, &s.id, "render", &sink(&sc.events), &Cancel::new()).await.unwrap_err();
    assert!(err.contains("Install engine"), "{err}");
    assert_eq!(store::load_song(&sc.ctx.data_dir, &s.id).unwrap().stages.render, ERROR);
    // The UI still hears about it, once.
    let events = sc.events.lock().unwrap();
    let errors: Vec<&ProgressEvent> = events.iter().filter(|e| e.event == "error").collect();
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].stage, "render");
    assert!(errors[0].message.contains("Install engine"));
}

#[tokio::test]
async fn check_reads_the_engines_report() {
    let sc = scratch("check");
    let v = engine::check(&sc.ctx, &Settings::default()).await.expect("a report");
    assert_eq!(v["problems"][0], "this is the fake engine");
    assert!(v["python"].as_str().unwrap().starts_with('3'));
    std::fs::remove_file(engine::venv_python(&sc.ctx.data_dir)).unwrap();
    assert!(engine::check(&sc.ctx, &Settings::default()).await.is_none());
}
