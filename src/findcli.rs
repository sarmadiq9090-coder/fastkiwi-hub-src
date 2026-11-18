//! File search in the bar through InstaFind's own read-only command line (InstaFind 1.2.0+:
//! `bahth.exe --json -n <max> -- <query>`, which reads its saved index and prints JSON).
//! Only an InstaFind exe that detection found is run, with no shell, no console window and
//! only these fixed flags; the query is one argument after `--`, so it is never parsed as an
//! option. Results are shown as names and folders; opening one is the person's click.

use serde::Deserialize;
use std::io::Read;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// The first InstaFind with the command line.
pub const MIN_VERSION: &str = "1.2.0";
const TIMEOUT: Duration = Duration::from_secs(4);
const MAX_OUT: usize = 512 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, serde::Serialize)]
pub struct Hit {
    pub path: String,
    pub name: String,
    #[serde(default)]
    pub is_dir: bool,
}

fn run(exe: &str, args: &[&str]) -> Result<Vec<u8>, String> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let mut child = Command::new(exe)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .map_err(|e| e.to_string())?;
    let mut out = child.stdout.take().ok_or("no stdout")?;
    let reader = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = out.by_ref().take(MAX_OUT as u64 + 1).read_to_end(&mut buf);
        buf
    });
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if start.elapsed() > TIMEOUT => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("InstaFind took too long".into());
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(20)),
            Err(e) => return Err(e.to_string()),
        }
    }
    let buf = reader.join().map_err(|_| "reader failed")?;
    if buf.len() > MAX_OUT {
        return Err("too much output".into());
    }
    Ok(buf)
}

/// `bahth 1.2.0` → `1.2.0`.
pub fn parse_version(out: &str) -> Option<String> {
    let v = out.trim().strip_prefix("bahth")?.trim();
    crate::updates::Version::parse(v).map(|_| v.to_string())
}

/// The version InstaFind reports, when it has the command line.
pub fn version(exe: &str) -> Option<String> {
    let out = run(exe, &["--version"]).ok()?;
    parse_version(&String::from_utf8_lossy(&out))
}

/// Does `version` have the command line?
pub fn supports(version: &str) -> bool {
    match (
        crate::updates::Version::parse(version),
        crate::updates::Version::parse(MIN_VERSION),
    ) {
        (Some(v), Some(min)) => v >= min,
        _ => false,
    }
}

pub fn parse_hits(json: &str) -> Result<Vec<Hit>, String> {
    let mut v: Vec<Hit> = serde_json::from_str(json.trim_start_matches('\u{feff}'))
        .map_err(|e| e.to_string())?;
    v.retain(|h| !h.path.is_empty() && h.path.len() < 1024 && !h.path.chars().any(char::is_control));
    Ok(v)
}

/// Search with InstaFind's saved index.
pub fn search(exe: &str, query: &str, max: usize) -> Result<Vec<Hit>, String> {
    let q: String = query.trim().chars().take(200).collect();
    if q.is_empty() {
        return Ok(Vec::new());
    }
    let n = max.clamp(1, 20).to_string();
    let out = run(exe, &["--json", "-n", &n, "--", &q])?;
    let text = String::from_utf8_lossy(&out);
    if text.trim().is_empty() {
        return Ok(Vec::new());
    }
    parse_hits(&text)
}
