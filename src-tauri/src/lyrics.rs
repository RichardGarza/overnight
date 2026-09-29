//! The lyricist: the Claude CLI in headless mode.
//!
//! We run `claude -p --output-format stream-json`, feed the prompt on stdin,
//! and read its event stream line by line. No tools at all: this is pure
//! writing, so the only thing we care about is the final `result` event,
//! which carries the song as JSON. The parsing is lenient because a model
//! in a hurry wraps the JSON in a code fence, adds a closing remark, or puts
//! a straight double quote inside a lyric line.

use std::path::Path;
use std::process::Stdio;
use std::time::{Duration, Instant};

use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

use crate::model::{Persona, Settings};
use crate::paths;

/// Hard stop for one Claude run (SPEC §4.2).
pub const TIMEOUT_SECS: u64 = 240;

/// What Claude hands back, already tidied.
#[derive(Debug, Clone, PartialEq)]
pub struct LyricsOutput {
    pub title: String,
    pub tags: String,
    pub lyrics: String,
    pub bpm: u32,
    pub key: String,
    pub duration_sec: u32,
    pub cover_mood: String,
}

// ------------------------------------------------------------------- prompt

/// The section list for a target length. The spec names three shapes; the
/// gaps between them go to the nearer shape.
pub fn structure_for(duration_sec: u32) -> &'static str {
    if duration_sec <= 105 {
        "[verse] / [chorus] / [verse] / [chorus]"
    } else if duration_sec < 195 {
        "[intro] / [verse] / [chorus] / [verse] / [chorus] / [bridge] / [chorus]"
    } else {
        "[intro] / [verse] / [chorus] / [verse] / [chorus] / [verse] / [chorus] / [bridge] / [chorus]"
    }
}

pub fn build_prompt(persona: &Persona, theme: &str, mood: &str, duration_sec: u32) -> String {
    let mood_line = if mood.trim().is_empty() {
        "MOOD: whatever the theme calls for, in the artist's lane".to_string()
    } else {
        format!("MOOD: {}", mood.trim())
    };
    let base_tags = persona.tags.trim();
    format!(
        r#"You are the songwriter for an artist. Write one complete, original song.

THE ARTIST
Name: {name}
Tagline: {tagline}
Voice: {voice}
Base style tags: {base_tags}
Usual themes: {themes}
How they write: {voice_notes}
Rules: {rules}

THIS SONG
THEME: {theme}
{mood_line}
TARGET LENGTH: about {duration_sec} seconds.
STRUCTURE for that length: {structure}

HOW TO WRITE IT
- Melodic-rap density: 8 to 12 syllables a bar, 12 to 16 bars a verse. A hook is 4 to 8 bars and repeats one plain line, the line that lands the theme, so the listener can sing it back after one listen.
- Concrete specifics over vague feeling: street names, times of night, brands, weather, what was on the screen. Flexes that turn into confessions.
- The hook must land on the theme. Every verse should earn the hook from a new angle.
- Original words only. Never quote, paraphrase or borrow from any existing song. No real people's names, no artist names, no brand-name artists in the tags.
- Keep it singable: short lines, natural stress, no tongue-twisters.

OUTPUT FORMAT (the music model reads it literally)
- Section tags on their own line, lower case, in square brackets: [intro], [verse], [pre-chorus], [chorus], [bridge], [outro]. One blank line between sections.
- Plain lines of lyrics under each tag. No chords, no timestamps, no bar numbers, no parentheses: the model sings everything it sees, so ad-libs go on their own short line or are left out.
- "tags" is one comma-separated style description: start with the base style tags exactly as given, then add 3 to 6 song-specific descriptors (an instrument, a texture, a mood, a tempo feel). Never an artist's name.
- "bpm" is a whole number and "key" a plain key name like "F minor". Both must agree with the tags.
- "durationSec" is your honest estimate of the finished length, close to the target.
- "coverMood" is three to six words for the cover art.

Reply with ONLY this JSON object, no code fence, no prose before or after. Inside string values use typographic quotes (“ ” ‘ ’), never a straight double quote, and write line breaks as \n.
{{"title":"...","tags":"...","lyrics":"[intro]\n...\n\n[verse]\n...","bpm":82,"key":"F minor","durationSec":{duration_sec},"coverMood":"..."}}"#,
        name = persona.name.trim(),
        tagline = persona.tagline.trim(),
        voice = persona.voice.trim(),
        themes = persona.themes.trim(),
        voice_notes = persona.voice_notes.trim(),
        rules = persona.rules.trim(),
        theme = theme.trim(),
        structure = structure_for(duration_sec),
    )
}

// ---------------------------------------------------------------------- run

/// Runs Claude and streams status through `on_status(message, detail)`.
///
/// If the installed CLI is old enough to reject one of the nice-to-have
/// flags, run once more with only the flags headless mode has always had.
pub async fn run<F>(bin: &Path, cwd: &Path, prompt: &str, settings: &Settings, mut on_status: F) -> Result<String, String>
where
    F: FnMut(&str, Option<String>) + Send,
{
    match run_once(bin, cwd, prompt, settings, false, &mut on_status).await {
        Err(e) if looks_like_rejected_flag(&e) => {
            on_status("Claude is writing", Some("Older CLI, retrying with basic options".into()));
            run_once(bin, cwd, prompt, settings, true, &mut on_status).await
        }
        other => other,
    }
}

pub fn looks_like_rejected_flag(err: &str) -> bool {
    let e = err.to_lowercase();
    ["unknown option", "unknown argument", "unexpected argument", "unrecognized option", "unrecognized argument"]
        .iter()
        .any(|needle| e.contains(needle))
}

/// One attempt. Returns the text of Claude's final message.
async fn run_once<F>(bin: &Path, cwd: &Path, prompt: &str, settings: &Settings, minimal: bool, on_status: &mut F) -> Result<String, String>
where
    F: FnMut(&str, Option<String>) + Send,
{
    let env = paths::shell_env().await;

    let mut cmd = tokio::process::Command::new(bin);
    cmd.arg("-p").args(["--output-format", "stream-json"]).arg("--verbose");
    if !minimal {
        // No tools: this is pure writing. Empty allowedTools plus the explicit
        // disallow list means an over-eager model cannot go looking things up
        // or write the song to a file instead of replying with it.
        cmd.args(["--allowedTools", ""])
            .args(["--disallowedTools", "Bash,Edit,Write,NotebookEdit"])
            // Ignore whatever MCP servers are configured: faster start, less context.
            .arg("--strict-mcp-config")
            .args(["--max-turns", "4"]);
        let effort = settings.claude_effort.trim();
        if !effort.is_empty() && effort != "default" {
            cmd.args(["--effort", effort]);
        }
    }
    // settings.json: claudeModel = "opus" (etc.) to change it, or "default"
    // to use whatever the CLI is set to. Empty = sonnet, fast and light on the plan.
    match settings.claude_model.trim() {
        "" => {
            cmd.args(["--model", "sonnet"]);
        }
        "default" => {}
        other => {
            cmd.args(["--model", other]);
        }
    }
    cmd.current_dir(cwd)
        .env("PATH", &env.path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);

    let mut child = cmd.spawn().map_err(|e| format!("Couldn't start the Claude CLI at {}: {e}", bin.display()))?;

    // Prompt goes in on stdin: no argument-length or quoting problems.
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(prompt.as_bytes()).await.map_err(|e| format!("Couldn't send the prompt to Claude: {e}"))?;
        drop(stdin);
    }

    let stdout = child.stdout.take().ok_or("no stdout from Claude")?;
    let stderr = child.stderr.take().ok_or("no stderr from Claude")?;

    // Drain stderr in the background so a chatty CLI can't fill the pipe and stall.
    let stderr_task = tokio::spawn(async move {
        let mut lines = BufReader::new(stderr).lines();
        let mut tail: Vec<String> = Vec::new();
        while let Ok(Some(l)) = lines.next_line().await {
            if !l.trim().is_empty() {
                tail.push(l);
                if tail.len() > 12 {
                    tail.remove(0);
                }
            }
        }
        tail.join("\n")
    });

    let deadline = Instant::now() + Duration::from_secs(TIMEOUT_SECS);
    let mut lines = BufReader::new(stdout).lines();

    let mut result_text: Option<String> = None;
    let mut result_error: Option<String> = None;
    // Every text block the model produced, in order. The song is supposed to
    // be the final message, but a model in a hurry sometimes files the JSON
    // and then adds a closing remark, which would otherwise be all we see.
    let mut texts: Vec<String> = Vec::new();
    let mut announced = false;

    on_status("Claude is writing", Some("Reading the brief".into()));

    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            let _ = child.kill().await;
            return Err(format!("Claude ran past the {TIMEOUT_SECS}-second limit. Try again."));
        }
        let line = match tokio::time::timeout(remaining, lines.next_line()).await {
            Err(_) => continue, // loop re-checks the deadline
            Ok(Err(e)) => return Err(format!("Lost the connection to Claude: {e}")),
            Ok(Ok(None)) => break,
            Ok(Ok(Some(l))) => l,
        };
        let Ok(ev) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        match ev.get("type").and_then(|t| t.as_str()).unwrap_or("") {
            "assistant" => {
                let blocks = ev.pointer("/message/content").and_then(|c| c.as_array()).cloned().unwrap_or_default();
                for b in blocks {
                    match b.get("type").and_then(|t| t.as_str()).unwrap_or("") {
                        "text" => {
                            if let Some(t) = b.get("text").and_then(|t| t.as_str()) {
                                if !t.trim().is_empty() {
                                    texts.push(t.to_string());
                                    if !announced && t.contains("\"lyrics\"") {
                                        announced = true;
                                        on_status("Claude is writing", Some("Lyrics coming in".into()));
                                    }
                                }
                            }
                        }
                        // If the model hands the song to some tool instead of
                        // replying with it, the copy is still a song. Keep it.
                        "tool_use" => {
                            if let Some(obj) = b.get("input").and_then(|i| i.as_object()) {
                                for v in obj.values() {
                                    if let Some(t) = v.as_str() {
                                        if t.contains("\"lyrics\"") {
                                            texts.push(t.to_string());
                                        }
                                    }
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
            "result" => {
                let is_error = ev.get("is_error").and_then(|b| b.as_bool()).unwrap_or(false);
                let text = ev.get("result").and_then(|r| r.as_str()).unwrap_or("").to_string();
                if is_error {
                    let subtype = ev.get("subtype").and_then(|s| s.as_str()).unwrap_or("error");
                    result_error = Some(if text.is_empty() { subtype.to_string() } else { text });
                } else {
                    result_text = Some(text);
                }
                break;
            }
            _ => {}
        }
    }

    let status = tokio::time::timeout(Duration::from_secs(10), child.wait()).await;
    let stderr_tail = tokio::time::timeout(Duration::from_secs(2), stderr_task).await.ok().and_then(|r| r.ok()).unwrap_or_default();

    if let Some(e) = result_error {
        return Err(explain_failure(&e, &stderr_tail));
    }

    let last_text = texts.last().cloned().unwrap_or_default();
    let text = match result_text {
        Some(t) if !t.trim().is_empty() => t,
        _ if !last_text.trim().is_empty() => last_text,
        _ => {
            let code = match status {
                Ok(Ok(s)) => s.code().map(|c| c.to_string()).unwrap_or_else(|| "signal".into()),
                _ => "unknown".into(),
            };
            return Err(explain_failure(&format!("Claude exited (code {code}) without a song."), &stderr_tail));
        }
    };

    // The final message first, then earlier ones, newest to oldest: take the
    // first that really is a song. Return the raw text so the caller can
    // keep it as a fixture; parsing happens in `parse_reply`.
    std::iter::once(&text)
        .chain(texts.iter().rev())
        .find(|t| extract_song(t).is_some())
        .cloned()
        .ok_or_else(|| format!("Claude replied, but not with a song I could parse. It started: \"{}\"", truncate(text.trim(), 160)))
}

fn explain_failure(msg: &str, stderr_tail: &str) -> String {
    let all = format!("{msg}\n{stderr_tail}").to_lowercase();
    if all.contains("/login") || all.contains("not logged in") || all.contains("invalid api key") || all.contains("authentication") {
        return "Claude CLI isn't signed in. Open a terminal, run `claude`, then `/login`, and try again.".into();
    }
    if all.contains("usage limit") || all.contains("rate limit") || all.contains("limit reached") {
        return format!("Claude says you've hit a usage limit: {}", truncate(msg.trim(), 200));
    }
    let mut out = truncate(msg.trim(), 300);
    if !stderr_tail.trim().is_empty() {
        out.push_str(&format!(" - {}", truncate(stderr_tail.trim(), 300)));
    }
    out
}

pub fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let cut: String = s.chars().take(max.saturating_sub(1)).collect();
    format!("{}\u{2026}", cut.trim_end())
}

// ------------------------------------------------------------------ parsing

/// Find the first JSON *object* in a blob of text. Handles a bare object,
/// a ```json fence, and prose or log lines around the object, and mends the
/// usual model slips (a straight quote inside a string, a raw line break).
pub fn extract_json(text: &str) -> Option<Value> {
    extract_json_where(text, &|_| true)
}

/// The song in a reply: the object that has a "lyrics" string.
pub fn extract_song(text: &str) -> Option<Value> {
    extract_json_where(text, &|v| v.get("lyrics").map(|l| l.is_string()).unwrap_or(false))
}

fn extract_json_where(text: &str, wanted: &dyn Fn(&Value) -> bool) -> Option<Value> {
    if let Some(v) = extract_json_strict(text, wanted) {
        return Some(v);
    }
    extract_json_strict(&repair_json(text), wanted)
}

fn extract_json_strict(text: &str, wanted: &dyn Fn(&Value) -> bool) -> Option<Value> {
    let t = text.trim();
    if let Ok(v) = serde_json::from_str::<Value>(t) {
        if v.is_object() && wanted(&v) {
            return Some(v);
        }
    }
    for (i, _) in t.match_indices('{') {
        let mut stream = serde_json::Deserializer::from_str(&t[i..]).into_iter::<Value>();
        if let Some(Ok(v)) = stream.next() {
            if v.is_object() && wanted(&v) {
                return Some(v);
            }
        }
    }
    None
}

/// Escape what JSON strings can't hold raw. A `"` inside a string only counts
/// as the closing quote when what follows looks like JSON structure
/// (`:` `}` `]`, end of text, or `,` and then the next string / object / array).
pub fn repair_json(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let next_non_space = |from: usize| -> Option<(usize, char)> { (from..chars.len()).find(|&j| !chars[j].is_whitespace()).map(|j| (j, chars[j])) };
    let mut out = String::with_capacity(text.len() + 64);
    let mut in_string = false;
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if !in_string {
            in_string = c == '"';
            out.push(c);
            i += 1;
            continue;
        }
        match c {
            '\\' => {
                out.push(c);
                if let Some(&n) = chars.get(i + 1) {
                    out.push(n);
                    i += 1;
                }
            }
            '"' => {
                let closes = match next_non_space(i + 1) {
                    None => true,
                    Some((_, ':')) | Some((_, '}')) | Some((_, ']')) => true,
                    Some((j, ',')) => matches!(next_non_space(j + 1), Some((_, '"')) | Some((_, '{')) | Some((_, '['))),
                    _ => false,
                };
                if closes {
                    in_string = false;
                    out.push('"');
                } else {
                    out.push_str("\\\"");
                }
            }
            '\n' => out.push_str("\\n"),
            '\r' => {}
            '\t' => out.push(' '),
            _ => out.push(c),
        }
        i += 1;
    }
    out
}

/// Bring the lyrics into the shape of SPEC §3.2 whatever the model did:
/// `[Verse 1]` becomes `[verse]`, parenthesised ad-libs are dropped, stray
/// blank lines collapse to one between sections.
pub fn tidy_lyrics(text: &str) -> String {
    let mut out: Vec<String> = Vec::new();
    for raw in text.replace("\r\n", "\n").lines() {
        let line = raw.trim();
        if line.starts_with('[') && line.ends_with(']') && line.len() > 2 {
            let inner = line[1..line.len() - 1].to_lowercase();
            // Keep the word, drop numbering and decorations: "verse 2" -> "verse".
            let tag: String = inner
                .split(|c: char| !c.is_ascii_alphabetic() && c != '-')
                .find(|w| !w.is_empty())
                .unwrap_or("verse")
                .to_string();
            let tag = match tag.as_str() {
                "prechorus" | "pre" => "pre-chorus".to_string(),
                "hook" | "refrain" => "chorus".to_string(),
                other => other.to_string(),
            };
            if let Some(last) = out.last() {
                if !last.is_empty() {
                    out.push(String::new());
                }
            }
            out.push(format!("[{tag}]"));
            continue;
        }
        let stripped = strip_parens(line);
        let stripped = stripped.trim();
        if stripped.is_empty() {
            // A blank between lyric lines is kept (one), a blank right under
            // a section tag is not: the tag and its first line stay together.
            if matches!(out.last(), Some(l) if !l.is_empty() && !l.starts_with('[')) {
                out.push(String::new());
            }
            continue;
        }
        out.push(stripped.to_string());
    }
    while matches!(out.last(), Some(l) if l.is_empty()) {
        out.pop();
    }
    out.join("\n")
}

fn strip_parens(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut depth = 0u32;
    for c in line.chars() {
        match c {
            '(' => depth += 1,
            ')' if depth > 0 => depth -= 1,
            _ if depth == 0 => out.push(c),
            _ => {}
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Turn Claude's reply into a song, forgiving missing or odd fields. The
/// persona and the request fill anything the model left out.
pub fn parse_reply(text: &str, persona: &Persona, theme: &str, duration_sec: u32) -> Result<LyricsOutput, String> {
    let v = extract_song(text).ok_or_else(|| format!("Claude replied, but not with a song I could parse. It started: \"{}\"", truncate(text.trim(), 160)))?;
    let s = |k: &str| -> Option<String> { v.get(k).and_then(|x| x.as_str()).map(|x| x.trim().to_string()).filter(|x| !x.is_empty() && x != "null") };
    let num = |k: &str| -> Option<f64> { v.get(k).and_then(|x| x.as_f64().or_else(|| x.as_str().and_then(|t| t.trim().parse().ok()))) };

    let lyrics = tidy_lyrics(&s("lyrics").unwrap_or_default());
    if lyrics.lines().filter(|l| !l.starts_with('[') && !l.trim().is_empty()).count() < 4 {
        return Err("Claude returned a song with almost no lyrics.".into());
    }
    let title = s("title").map(|t| truncate(&t, 80)).unwrap_or_else(|| title_from(theme));
    let base = persona.tags.trim();
    let tags = match s("tags") {
        Some(t) if base.is_empty() || t.to_lowercase().contains(&base.to_lowercase()[..base.len().min(20)]) => t,
        Some(t) => format!("{base}, {t}"),
        None => base.to_string(),
    };
    let bpm = num("bpm").map(|b| b.round() as u32).filter(|b| (40..=220).contains(b)).unwrap_or_else(|| bpm_from_tags(&tags).unwrap_or(82));
    let key = s("key").unwrap_or_else(|| "minor".into());
    let duration = num("durationSec").map(|d| d.round() as u32).filter(|d| (20..=600).contains(d)).unwrap_or(duration_sec);
    let cover_mood = s("coverMood").map(|m| truncate(&m, 60)).unwrap_or_default();
    Ok(LyricsOutput { title, tags, lyrics, bpm, key, duration_sec: duration, cover_mood })
}

/// "she saw the message at 2am" -> "She Saw The Message At 2am" (first 6 words).
fn title_from(theme: &str) -> String {
    let words: Vec<String> = theme
        .split_whitespace()
        .take(6)
        .map(|w| {
            let mut c = w.chars();
            match c.next() {
                Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
                None => String::new(),
            }
        })
        .collect();
    if words.is_empty() {
        "Untitled".into()
    } else {
        words.join(" ")
    }
}

/// "..., 82 bpm, ..." -> 82
pub fn bpm_from_tags(tags: &str) -> Option<u32> {
    tags.split(',').find_map(|part| {
        let mut it = part.split_whitespace();
        let n: u32 = it.next()?.parse().ok()?;
        it.next().filter(|w| w.eq_ignore_ascii_case("bpm")).map(|_| n)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn persona() -> Persona {
        crate::store::default_persona().unwrap()
    }

    #[test]
    fn extracts_json_from_messy_replies() {
        assert!(extract_json(r#"{"a":1}"#).is_some());
        assert!(extract_json("Here you go:\n```json\n{\"a\":{\"b\":[1,2]}}\n```\nEnjoy!").is_some());
        assert!(extract_json("log {not json} then {\"ok\":true} trailing }").unwrap()["ok"].as_bool().unwrap());
        assert!(extract_json("no json here").is_none());
        assert!(extract_json("[1,2,3]").is_none());
    }

    #[test]
    fn extract_song_wants_lyrics() {
        let text = "{\"title\":\"no lyrics\"} and then {\"title\":\"yes\",\"lyrics\":\"[verse]\\nhi\"}";
        assert_eq!(extract_song(text).unwrap()["title"], "yes");
    }

    #[test]
    fn mends_straight_quotes_and_raw_line_breaks() {
        let broken = "{\"title\":\"Left \"On\" Read\",\"tags\":\"a, b\",\"lyrics\":\"[verse]\nshe said \"no\" and left\nsecond line\",\"bpm\":82}";
        assert!(serde_json::from_str::<Value>(broken).is_err());
        let v = extract_song(broken).expect("mended");
        assert_eq!(v["title"], "Left \"On\" Read");
        assert!(v["lyrics"].as_str().unwrap().contains("said \"no\" and left\nsecond line"));
    }

    #[test]
    fn repair_leaves_good_json_alone() {
        let good = r#"{"a":"x \"quoted\" y","b":["1","2"],"c":{"d":"e"}}"#;
        assert_eq!(repair_json(good), good);
    }

    #[test]
    fn tidies_lyrics_into_ace_step_shape() {
        let raw = "[Intro]\n\n\nYeah (yeah)\n[Verse 1]\nline one (ad lib)\n  line two  \n\n\n[Hook]\nhook line\n[Pre-Chorus]\nx\n[Outro]\n\n";
        let t = tidy_lyrics(raw);
        assert_eq!(t, "[intro]\nYeah\n\n[verse]\nline one\nline two\n\n[chorus]\nhook line\n\n[pre-chorus]\nx\n\n[outro]");
    }

    #[test]
    fn parses_a_reply_and_fills_gaps() {
        let p = persona();
        let reply = r#"{"title":"Left On Read","tags":"melodic rap, r&b, trap soul, moody, atmospheric pads, sparse 808s, soft hi-hats, late night, rainy city, male vocal, autotune, 82 bpm, minor key, rhodes, rain on the windshield","lyrics":"[verse]\na\nb\nc\nd\n\n[chorus]\ne\nf","bpm":"82","key":"F minor","durationSec":150,"coverMood":"blue rain"}"#;
        let out = parse_reply(reply, &p, "she never answered", 150).unwrap();
        assert_eq!(out.title, "Left On Read");
        assert_eq!(out.bpm, 82);
        assert_eq!(out.key, "F minor");
        assert!(out.tags.starts_with("melodic rap"));
        assert_eq!(out.cover_mood, "blue rain");

        // Tags that forgot the base get it prepended; a bogus bpm falls back to the tags.
        let thin = r#"{"tags":"rhodes, rain","lyrics":"[verse]\na\nb\nc\nd","bpm":9000}"#;
        let out = parse_reply(thin, &p, "she saw the message at 2am and never answered", 90).unwrap();
        assert!(out.tags.starts_with("melodic rap, r&b"));
        assert!(out.tags.ends_with("rhodes, rain"));
        assert_eq!(out.bpm, 82);
        assert_eq!(out.title, "She Saw The Message At 2am");
        assert_eq!(out.duration_sec, 90);

        assert!(parse_reply(r#"{"lyrics":"[verse]\nonly one line"}"#, &p, "t", 90).is_err());
        assert!(parse_reply("nothing here", &p, "t", 90).is_err());
    }

    #[test]
    fn bpm_is_read_from_tags() {
        assert_eq!(bpm_from_tags("moody, 82 bpm, minor key"), Some(82));
        assert_eq!(bpm_from_tags("moody, 140 BPM"), Some(140));
        assert_eq!(bpm_from_tags("moody"), None);
    }

    #[test]
    fn structure_follows_the_length() {
        assert!(structure_for(90).starts_with("[verse]"));
        assert!(structure_for(150).contains("[bridge]"));
        assert_eq!(structure_for(150).matches("[verse]").count(), 2);
        assert_eq!(structure_for(210).matches("[verse]").count(), 3);
    }

    #[test]
    fn prompt_carries_the_persona_theme_and_rules() {
        let p = persona();
        let prompt = build_prompt(&p, "she saw the message at 2am", "Heartbreak", 150);
        assert!(prompt.contains("Name: Overnight"));
        assert!(prompt.contains("THEME: she saw the message at 2am"));
        assert!(prompt.contains("MOOD: Heartbreak"));
        assert!(prompt.contains("[bridge]"));
        assert!(prompt.contains("8 to 12 syllables"));
        assert!(prompt.contains("\"durationSec\":150"));
        assert!(prompt.contains(p.tags.as_str()));
        let no_mood = build_prompt(&p, "x", "  ", 90);
        assert!(no_mood.contains("MOOD: whatever"));
    }

    #[test]
    fn spots_a_cli_that_rejected_a_flag() {
        assert!(looks_like_rejected_flag("error: unknown option '--strict-mcp-config'"));
        assert!(!looks_like_rejected_flag("Claude CLI isn't signed in."));
    }

    /// What a real run of the claude CLI returned for the default persona and
    /// the sample theme (captured by `e2e_claude_writes_a_song`). Parsing it
    /// is part of the normal test run.
    #[test]
    fn parses_the_recorded_claude_reply() {
        let text = include_str!("../tests/fixtures/claude-lyrics-reply.txt");
        let out = parse_reply(text, &persona(), "she saw the message at 2am and never answered", 150).unwrap();
        assert!(!out.title.is_empty());
        assert!(out.tags.starts_with("melodic rap"));
        assert!(out.lyrics.contains("[chorus]"));
        assert!(out.lyrics.lines().filter(|l| !l.starts_with('[') && !l.is_empty()).count() >= 20);
        assert!(out.bpm > 0);
    }

    /// Real end-to-end run against the installed `claude` CLI (uses your
    /// Claude plan). Not part of the normal test run:
    ///   cargo test --lib e2e_claude -- --ignored --nocapture
    /// Saves the raw reply as the fixture the test above reads.
    #[tokio::test]
    #[ignore]
    async fn e2e_claude_writes_a_song() {
        let p = persona();
        let theme = "she saw the message at 2am and never answered";
        let prompt = build_prompt(&p, theme, "Late night drive", 150);
        let bin = paths::find_bin("claude", "").await.expect("claude on PATH");
        let cwd = std::env::temp_dir().join("overnight-e2e");
        std::fs::create_dir_all(&cwd).unwrap();
        let settings = Settings::default();
        let t0 = Instant::now();
        let text = run(&bin, &cwd, &prompt, &settings, move |msg, detail| {
            println!("{:>4}s {msg} {}", t0.elapsed().as_secs(), detail.unwrap_or_default());
        })
        .await
        .expect("a reply");
        println!("took {:.1}s", t0.elapsed().as_secs_f64());
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/claude-lyrics-reply.txt");
        std::fs::create_dir_all(fixture.parent().unwrap()).unwrap();
        std::fs::write(&fixture, &text).unwrap();
        let out = parse_reply(&text, &p, theme, 150).expect("a song");
        println!("title: {}\ntags: {}\nbpm: {} key: {} duration: {}\ncover: {}\n\n{}", out.title, out.tags, out.bpm, out.key, out.duration_sec, out.cover_mood, out.lyrics);
        assert!(out.lyrics.contains("[chorus]"));
    }
}
