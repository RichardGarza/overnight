//! Shared data types: settings, the persona, songs, and the events the UI
//! listens for. Everything here is JSON in camelCase on the wire (SPEC §2),
//! and every settings field has a default so an old or half-written
//! settings.json still loads.

use serde::{Deserialize, Serialize};
use serde_json::Value;

// ----------------------------------------------------------------- settings

/// `settings.json` in the app data folder (SPEC §2.1).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct Settings {
    /// "local" (ACE-Step in the venv) or "fal" (fal-ai/ace-step, needs `fal_key`).
    pub engine: String,
    pub fal_key: String,
    /// Absolute path to the `claude` binary. Empty = auto-detect.
    pub claude_bin: String,
    /// Empty = "sonnet"; "opus"; "default" = whatever the CLI is set to.
    pub claude_model: String,
    /// `--effort` for the lyricist: low | medium | high | default.
    pub claude_effort: String,
    /// Absolute path to `uv`. Empty = auto-detect.
    pub uv_bin: String,
    /// auto | mps | cpu. auto = mps on Apple Silicon.
    pub device: String,
    /// ACE-Step inference steps (27 fast, 60 quality).
    pub steps: u32,
    pub guidance: f64,
    pub default_duration_sec: u32,
    /// Run the voice-conversion stage (needs `voice_model.pth`).
    pub convert: bool,
    pub voice_model: VoiceModel,
    /// Vocal level relative to the beat in the mix.
    pub vocals_gain_db: f64,
    pub mp3_bitrate: u32,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            engine: "local".into(),
            fal_key: String::new(),
            claude_bin: String::new(),
            claude_model: String::new(),
            claude_effort: "medium".into(),
            uv_bin: String::new(),
            device: "auto".into(),
            steps: 60,
            guidance: 15.0,
            default_duration_sec: 150,
            convert: false,
            voice_model: VoiceModel::default(),
            vocals_gain_db: 0.0,
            mp3_bitrate: 320,
        }
    }
}

/// The RVC model the user pointed the app at, and its conversion knobs.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct VoiceModel {
    /// Absolute path to the `.pth`. Empty = none.
    pub pth: String,
    /// Absolute path to the `.index`. Empty = none.
    pub index: String,
    /// Semitones.
    pub pitch: i32,
    /// rmvpe | harvest | crepe | pm
    pub method: String,
    pub index_rate: f64,
    pub protect: f64,
    pub filter_radius: u32,
    pub rms_mix_rate: f64,
}

impl Default for VoiceModel {
    fn default() -> Self {
        Self {
            pth: String::new(),
            index: String::new(),
            pitch: 0,
            method: "rmvpe".into(),
            index_rate: 0.66,
            protect: 0.33,
            filter_radius: 3,
            rms_mix_rate: 0.25,
        }
    }
}

// ------------------------------------------------------------------ persona

/// Who is singing (SPEC §2.2). Edited on the Artist page; a snapshot is kept
/// in every song so re-mixing an old song keeps its original voice.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct Persona {
    pub name: String,
    pub tagline: String,
    pub voice: String,
    /// The style prompt every song starts from: genre, mood, instruments, bpm, key.
    pub tags: String,
    pub themes: String,
    pub voice_notes: String,
    pub rules: String,
    pub cover_style: String,
}

// --------------------------------------------------------------------- song

/// One `songs/<id>/song.json` (SPEC §2.3).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct Song {
    /// `YYYYMMDD-HHMMSS-xxxx`
    pub id: String,
    pub title: String,
    /// What the user typed.
    pub theme: String,
    /// The mood chip they picked; may be empty.
    pub mood: String,
    /// RFC 3339 local time.
    pub created_at: String,
    pub persona: Persona,
    /// ACE-Step format (SPEC §3.2).
    pub lyrics: String,
    /// Style prompt sent to the model.
    pub tags: String,
    pub bpm: u32,
    pub key: String,
    pub duration_sec: u32,
    /// Engine used for the render: "local" | "fal".
    pub engine: String,
    pub seed: u64,
    pub steps: u32,
    pub stages: Stages,
    /// Last error message, if any.
    pub error: Option<String>,
    pub files: SongFiles,
    pub actual_duration_sec: Option<f64>,
    /// Wall clock of the render stage.
    pub render_seconds: Option<f64>,
}

impl Default for Song {
    fn default() -> Self {
        Self {
            id: String::new(),
            title: String::new(),
            theme: String::new(),
            mood: String::new(),
            created_at: String::new(),
            persona: Persona::default(),
            lyrics: String::new(),
            tags: String::new(),
            bpm: 0,
            key: String::new(),
            duration_sec: 0,
            engine: "local".into(),
            seed: 0,
            steps: 0,
            stages: Stages::default(),
            error: None,
            files: SongFiles::default(),
            actual_duration_sec: None,
            render_seconds: None,
        }
    }
}

/// Stage status values. Kept as strings so the JSON reads plainly.
pub const PENDING: &str = "pending";
pub const RUNNING: &str = "running";
pub const DONE: &str = "done";
pub const ERROR: &str = "error";
pub const SKIPPED: &str = "skipped";

/// The pipeline stages in order. `lyrics` is Claude; the rest are the engine.
pub const STAGE_ORDER: [&str; 5] = ["lyrics", "render", "separate", "convert", "mix"];

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct Stages {
    pub lyrics: String,
    pub render: String,
    pub separate: String,
    pub convert: String,
    pub mix: String,
}

impl Default for Stages {
    fn default() -> Self {
        Self {
            lyrics: PENDING.into(),
            render: PENDING.into(),
            separate: PENDING.into(),
            convert: PENDING.into(),
            mix: PENDING.into(),
        }
    }
}

impl Stages {
    pub fn get(&self, stage: &str) -> Option<&str> {
        match stage {
            "lyrics" => Some(&self.lyrics),
            "render" => Some(&self.render),
            "separate" => Some(&self.separate),
            "convert" => Some(&self.convert),
            "mix" => Some(&self.mix),
            _ => None,
        }
    }

    /// Ignores unknown stage names (the engine's "setup" stage has no slot).
    pub fn set(&mut self, stage: &str, status: &str) {
        let slot = match stage {
            "lyrics" => &mut self.lyrics,
            "render" => &mut self.render,
            "separate" => &mut self.separate,
            "convert" => &mut self.convert,
            "mix" => &mut self.mix,
            _ => return,
        };
        *slot = status.to_string();
    }

    /// The stage currently marked running, if any.
    pub fn running(&self) -> Option<&'static str> {
        STAGE_ORDER.iter().copied().find(|s| self.get(s) == Some(RUNNING))
    }
}

/// File names relative to the song folder, once they exist.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct SongFiles {
    pub raw: Option<String>,
    pub vocals: Option<String>,
    pub instrumental: Option<String>,
    pub converted: Option<String>,
    #[serde(rename = "final")]
    pub final_: Option<String>,
    pub mp3: Option<String>,
    pub cover: Option<String>,
}

/// What `update_song` may change after the lyrics review.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct SongPatch {
    pub title: Option<String>,
    pub lyrics: Option<String>,
    pub tags: Option<String>,
    pub duration_sec: Option<u32>,
}

// -------------------------------------------------------------------- setup

/// What the Setup page shows (SPEC §4.3).
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct SetupStatus {
    pub data_dir: String,
    pub claude: Option<String>,
    pub uv: Option<String>,
    pub ffmpeg: Option<String>,
    /// The engine source dir the app will use; null when none was found.
    pub engine_source: Option<String>,
    pub venv: bool,
    /// The engine's own `check` report (SPEC §3.4), null without a venv.
    pub check: Option<Value>,
    pub installing: bool,
}

// ------------------------------------------------------------------- events

/// `overnight-progress`: one per engine protocol line (SPEC §3.3), plus the
/// lyrics stage from the Claude runner.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ProgressEvent {
    pub song_id: String,
    /// render | separate | convert | mix | setup | lyrics
    pub stage: String,
    /// start | progress | done | error | log
    pub event: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pct: Option<u32>,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// Milliseconds since the Unix epoch.
    pub at: i64,
}

/// `overnight-log`: a line of installer output.
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct LogEvent {
    pub line: String,
    pub at: i64,
}

pub fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_defaults_match_the_spec() {
        let s = Settings::default();
        assert_eq!(s.engine, "local");
        assert_eq!(s.claude_effort, "medium");
        assert_eq!(s.device, "auto");
        assert_eq!(s.steps, 60);
        assert_eq!(s.guidance, 15.0);
        assert_eq!(s.default_duration_sec, 150);
        assert!(!s.convert);
        assert_eq!(s.voice_model.method, "rmvpe");
        assert_eq!(s.voice_model.index_rate, 0.66);
        assert_eq!(s.voice_model.protect, 0.33);
        assert_eq!(s.voice_model.filter_radius, 3);
        assert_eq!(s.voice_model.rms_mix_rate, 0.25);
        assert_eq!(s.mp3_bitrate, 320);
    }

    #[test]
    fn settings_json_is_camel_case_and_missing_keys_use_defaults() {
        let s: Settings = serde_json::from_str(r#"{"engine":"fal","voiceModel":{"pitch":-2}}"#).unwrap();
        assert_eq!(s.engine, "fal");
        assert_eq!(s.voice_model.pitch, -2);
        assert_eq!(s.voice_model.method, "rmvpe");
        assert_eq!(s.steps, 60);
        let text = serde_json::to_string(&s).unwrap();
        assert!(text.contains("\"falKey\""));
        assert!(text.contains("\"defaultDurationSec\""));
        assert!(text.contains("\"indexRate\""));
        let empty: Settings = serde_json::from_str("{}").unwrap();
        assert_eq!(empty, Settings::default());
    }

    #[test]
    fn song_files_serialise_final_without_underscore() {
        let f = SongFiles { final_: Some("final.wav".into()), ..Default::default() };
        let text = serde_json::to_string(&f).unwrap();
        assert!(text.contains("\"final\":\"final.wav\""));
        assert!(!text.contains("final_"));
        let back: SongFiles = serde_json::from_str(&text).unwrap();
        assert_eq!(back.final_.as_deref(), Some("final.wav"));
    }

    #[test]
    fn stages_get_set_and_running() {
        let mut st = Stages::default();
        assert_eq!(st.running(), None);
        st.set("render", RUNNING);
        st.set("setup", RUNNING); // no slot, ignored
        assert_eq!(st.running(), Some("render"));
        assert_eq!(st.get("render"), Some(RUNNING));
        assert_eq!(st.get("nope"), None);
    }

    #[test]
    fn progress_event_omits_empty_optionals() {
        let e = ProgressEvent { song_id: "x".into(), stage: "render".into(), event: "start".into(), pct: None, message: "m".into(), detail: None, at: 1 };
        let text = serde_json::to_string(&e).unwrap();
        assert_eq!(text, r#"{"songId":"x","stage":"render","event":"start","message":"m","at":1}"#);
    }
}
