//! Finding `claude`, `python3`, `ffmpeg` and friends.
//!
//! A macOS app launched from Finder does not inherit the terminal's PATH, so
//! "works in my terminal" tools are invisible to it. We ask the user's login
//! shell for its PATH once, add the usual install locations, and search that.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use tokio::sync::OnceCell;

#[derive(Debug, Clone, Default)]
pub struct ShellEnv {
    /// PATH to hand to child processes (login-shell PATH + known install dirs).
    pub path: String,
}

static SHELL_ENV: OnceCell<ShellEnv> = OnceCell::const_new();

const MARKER: &str = "__ON_PATH__";

pub async fn shell_env() -> &'static ShellEnv {
    SHELL_ENV
        .get_or_init(|| async {
            let mut dirs: Vec<String> = Vec::new();

            if let Some(p) = login_shell_path().await {
                push_split(&mut dirs, &p);
            }
            if let Ok(p) = std::env::var("PATH") {
                push_split(&mut dirs, &p);
            }
            for d in well_known_dirs() {
                let s = d.to_string_lossy().to_string();
                if !dirs.contains(&s) {
                    dirs.push(s);
                }
            }
            ShellEnv {
                path: dirs.join(":"),
            }
        })
        .await
}

fn push_split(dirs: &mut Vec<String>, path: &str) {
    for d in path.split(':') {
        let d = d.trim();
        if !d.is_empty() && !dirs.iter().any(|x| x == d) {
            dirs.push(d.to_string());
        }
    }
}

/// Run the user's shell as an interactive login shell and read back $PATH.
async fn login_shell_path() -> Option<String> {
    if cfg!(windows) {
        return None;
    }
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".to_string());
    let script = format!("printf '\\n{MARKER}%s\\n' \"$PATH\"");

    let mut cmd = tokio::process::Command::new(&shell);
    cmd.args(["-l", "-i", "-c", &script])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);

    let out = tokio::time::timeout(Duration::from_secs(8), cmd.output())
        .await
        .ok()?
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    text.lines()
        .rev()
        .find_map(|l| l.trim().strip_prefix(MARKER).map(|s| s.trim().to_string()))
        .filter(|s| !s.is_empty())
}

fn home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

/// Places the Claude and Grok installers (and npm / Homebrew) put binaries.
fn well_known_dirs() -> Vec<PathBuf> {
    let mut v = Vec::new();
    if let Some(h) = home() {
        for rel in [
            ".local/bin",
            ".claude/local",
            ".claude/bin",
            ".grok/bin",
            ".bun/bin",
            ".npm-global/bin",
            ".homebrew/bin",
            ".volta/bin",
            "bin",
        ] {
            v.push(h.join(rel));
        }
        // nvm: newest node version first.
        let nvm = h.join(".nvm/versions/node");
        if let Ok(rd) = std::fs::read_dir(&nvm) {
            let mut versions: Vec<PathBuf> = rd.flatten().map(|e| e.path()).collect();
            versions.sort();
            versions.reverse();
            for ver in versions {
                v.push(ver.join("bin"));
            }
        }
    }
    for abs in ["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin", "/bin"] {
        v.push(PathBuf::from(abs));
    }
    v
}

fn is_executable(p: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(p)
            .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
    }
    #[cfg(not(unix))]
    {
        p.is_file()
    }
}

/// Resolve a binary: explicit override first, then the PATH search.
pub async fn find_bin(name: &str, override_path: &str) -> Option<PathBuf> {
    let o = override_path.trim();
    if !o.is_empty() {
        let p = expand_tilde(o);
        return is_executable(&p).then_some(p);
    }
    let env = shell_env().await;
    env.path
        .split(':')
        .map(|d| Path::new(d).join(name))
        .find(|p| is_executable(p))
}

pub fn expand_tilde(p: &str) -> PathBuf {
    if let Some(rest) = p.strip_prefix("~/") {
        if let Some(h) = home() {
            return h.join(rest);
        }
    }
    PathBuf::from(p)
}
