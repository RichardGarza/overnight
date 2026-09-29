//! The real thing, opt-in: `OVERNIGHT_E2E=1 cargo test --test engine_real -- --nocapture`.
//!
//! Drives `engine::run_pipeline` against the actual venv and models in the
//! app data folder (created by engine/setup.sh) and the engine source in
//! ../engine, exactly as the app does. Renders a 30-second song, separates
//! it and mixes it. The song stays in the library on purpose: it is the
//! first thing on the shelf when the app opens.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use overnight_lib::engine::{self, Cancel, Ctx};
use overnight_lib::model::{ProgressEvent, Song, DONE, SKIPPED};
use overnight_lib::store;

fn real_data_dir() -> PathBuf {
    let home = std::env::var_os("HOME").map(PathBuf::from).expect("HOME");
    home.join("Library/Application Support/com.overnight.studio")
}

#[tokio::test]
async fn renders_separates_and_mixes_a_real_song() {
    if std::env::var("OVERNIGHT_E2E").ok().as_deref() != Some("1") {
        eprintln!("skipped: set OVERNIGHT_E2E=1 to run the real engine");
        return;
    }
    let data_dir = real_data_dir();
    let engine_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../engine").canonicalize().unwrap();
    assert!(engine::has_venv(&data_dir), "no venv at {}; run engine/setup.sh first", data_dir.display());
    store::ensure_layout(&data_dir).unwrap();
    let path = format!("{}:/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin", std::env::var("PATH").unwrap_or_default());
    let ctx = Ctx { data_dir: data_dir.clone(), engine_dir, path };

    let song = Song {
        id: store::new_song_id(),
        title: "First Light".into(),
        theme: "the first song the machine ever made, at the end of a long night".into(),
        mood: "late night drive".into(),
        lyrics: "[verse]\nfirst light coming through the blinds again\nphone face down, I don't need to know who called\nI been up since the city went quiet\nwriting things I'll never say out loud\n\n[chorus]\nstill up, still up\nsun's coming and I'm still up\nif you're awake then you know what it's like\nstill up, still up".into(),
        tags: "melodic rap, r&b, trap soul, moody, atmospheric pads, sparse 808s, soft hi-hats, late night, male vocal, autotune, 82 bpm, minor key".into(),
        bpm: 82,
        key: "F minor".into(),
        duration_sec: 30,
        seed: 20260929,
        steps: 60,
        ..Default::default()
    };
    store::save_song(&data_dir, &song).unwrap();
    let song_dir = store::song_dir(&data_dir, &song.id).unwrap();

    let events: Arc<Mutex<Vec<ProgressEvent>>> = Arc::new(Mutex::new(Vec::new()));
    let sink = {
        let events = events.clone();
        move |e: ProgressEvent| {
            eprintln!("[{}] {} {} {}", e.stage, e.event, e.pct.map(|p| format!("{p}%")).unwrap_or_default(), e.message);
            events.lock().unwrap().push(e)
        }
    };
    let t0 = std::time::Instant::now();
    let out = engine::run_pipeline(&ctx, &song.id, "render", &sink, &Cancel::new()).await.expect("pipeline ok");
    eprintln!("real run took {:.1}s, render {:?}s, audio {:?}s", t0.elapsed().as_secs_f64(), out.render_seconds, out.actual_duration_sec);

    assert_eq!(out.stages.render, DONE);
    assert_eq!(out.stages.separate, DONE);
    assert_eq!(out.stages.convert, SKIPPED);
    assert_eq!(out.stages.mix, DONE);
    assert_eq!(out.error, None);
    for f in ["raw.wav", "vocals.wav", "instrumental.wav", "final.wav", "final.mp3", "cover.png", "engine.log"] {
        let p = song_dir.join(f);
        assert!(p.is_file(), "{f} missing");
        assert!(std::fs::metadata(&p).unwrap().len() > 1_000, "{f} is tiny");
    }
    let secs = out.actual_duration_sec.expect("duration read from raw.wav");
    assert!((25.0..=35.0).contains(&secs), "raw.wav is {secs} s, expected about 30");
    let ev = events.lock().unwrap();
    assert!(ev.iter().any(|e| e.stage == "render" && e.event == "progress"), "render progress events arrived");
    assert!(ev.iter().any(|e| e.stage == "mix" && e.event == "done"));
    eprintln!("song folder: {}", song_dir.display());
}
