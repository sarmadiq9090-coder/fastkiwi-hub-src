//! What the Hub does online, all from one worker thread: refresh the catalog from
//! fastkiwi.app, fetch icons of new entries, and check GitHub releases. Results are cached in
//! the Hub's own folder (`%LOCALAPPDATA%\FastKiwi\Hub`), so the Hub asks at most once a day by
//! itself and works offline from the cache or the copy inside the exe. Nothing fetched is
//! ever run: only JSON is parsed, and icons are shown as images.

use crate::catalog::{self, Catalog};
use crate::links;
use crate::net::{self, Host};
use crate::updates::{self, CacheEntry, Latest};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

const CATALOG_FILE: &str = "catalog-cache.json";
const SERIAL_FILE: &str = "catalog-serial.txt";
/// `https://fastkiwi.app/data/apps.json` and its minisign signature.
const CATALOG_PATH: &str = "/data/apps.json";
const SIG_PATH: &str = "/data/apps.json.minisig";
const UPDATES_FILE: &str = "updates-cache.json";
const ICONS_DIR: &str = "icons";
const MAX_RELEASES: usize = 1024 * 1024;
const MAX_ICON: usize = 256 * 1024;

pub fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

pub fn today() -> String {
    catalog::date_from_unix(now())
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct CatalogCache {
    checked: u64,
    #[serde(default)]
    etag: Option<String>,
    #[serde(default)]
    text: String,
    #[serde(default)]
    sig: String,
}

fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d)?;
    }
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(tmp, path)
}

fn read_catalog_cache(dir: &Path) -> CatalogCache {
    std::fs::read_to_string(dir.join(CATALOG_FILE))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

/// The catalog text and its signature cached from the site, if any.
pub fn cached_catalog(dir: Option<&Path>) -> Option<(String, String)> {
    let c = read_catalog_cache(dir?);
    (!c.text.is_empty() && !c.sig.is_empty()).then_some((c.text, c.sig))
}

/// The highest catalog serial this PC has accepted (rollback protection).
pub fn seen_serial(dir: Option<&Path>) -> u64 {
    dir.and_then(|d| std::fs::read_to_string(d.join(SERIAL_FILE)).ok())
        .and_then(|t| t.trim().parse().ok())
        .unwrap_or(0)
}

fn remember_serial(dir: Option<&Path>, serial: u64) {
    if let Some(d) = dir
        && serial > seen_serial(Some(d))
    {
        let _ = write_atomic(&d.join(SERIAL_FILE), serial.to_string().as_bytes());
    }
}

/// Is the catalog cache older than `max_age`?
pub fn catalog_due(dir: Option<&Path>, max_age: u64) -> bool {
    dir.is_none_or(|d| now().saturating_sub(read_catalog_cache(d).checked) >= max_age)
}

/// Ask fastkiwi.app for the signed catalog. `Ok(Some)`: a newer valid catalog with a good
/// signature, now cached. `Ok(None)`: unchanged. `Err`: unreachable or refused (the Hub keeps
/// what it has).
pub fn refresh_catalog(dir: Option<&Path>, min_serial: u64) -> Result<Option<Catalog>, String> {
    let mut cache = dir.map(read_catalog_cache).unwrap_or_default();
    let have = !cache.text.is_empty() && !cache.sig.is_empty();
    let etag = if have { cache.etag.clone() } else { None };
    let min = min_serial.max(seen_serial(dir));
    let r = net::get(Host::Site, CATALOG_PATH, etag.as_deref(), catalog::MAX_BYTES)?;
    cache.checked = now();
    let result = match r.status {
        304 => Ok(None),
        200 => {
            let text = String::from_utf8(r.body).map_err(|_| "not UTF-8".to_string());
            let sig = net::get(Host::Site, SIG_PATH, None, crate::trust::MAX_SIG)
                .and_then(|s| {
                    if s.status == 200 {
                        String::from_utf8(s.body).map_err(|_| "signature not UTF-8".into())
                    } else {
                        Err(format!("signature: HTTP {}", s.status))
                    }
                });
            match (text, sig) {
                (Ok(text), Ok(sig)) => match catalog::accept_signed(&text, &sig, &today(), min) {
                    Ok(c) => {
                        remember_serial(dir, c.serial);
                        let changed = text != cache.text;
                        cache.text = text;
                        cache.sig = sig;
                        cache.etag = r.etag;
                        Ok(changed.then_some(c))
                    }
                    Err(e) => Err(format!("refused: {e}")),
                },
                (Err(e), _) | (_, Err(e)) => Err(e),
            }
        }
        s => Err(format!("HTTP {s}")),
    };
    if let Some(d) = dir {
        let _ = write_atomic(
            &d.join(CATALOG_FILE),
            serde_json::to_string(&cache).unwrap_or_default().as_bytes(),
        );
    }
    result
}

/// Where an entry's downloaded icon is kept: `icons/<id>-<file name>`.
pub fn icon_path(dir: &Path, id: &str, icon: &str) -> Option<PathBuf> {
    let name = links::icon_file_name(icon)?;
    catalog::valid_id(id).then(|| dir.join(ICONS_DIR).join(format!("{id}-{name}")))
}

fn looks_like_icon(name: &str, b: &[u8]) -> bool {
    if name.to_ascii_lowercase().ends_with(".png") {
        return b.starts_with(b"\x89PNG\r\n\x1a\n");
    }
    let head = String::from_utf8_lossy(&b[..b.len().min(512)]).to_ascii_lowercase();
    let head = head.trim_start_matches('\u{feff}').trim_start();
    head.starts_with("<svg") || head.starts_with("<?xml")
}

/// Fetch the icon of an entry from fastkiwi.app, once.
pub fn fetch_icon(dir: &Path, id: &str, icon: &str) {
    let (Some(url), Some(out)) = (links::remote_icon(icon), icon_path(dir, id, icon)) else {
        return;
    };
    if out.is_file() {
        return;
    }
    let Ok(r) = net::get(Host::Site, url.path(), None, MAX_ICON) else {
        return;
    };
    let name = links::icon_file_name(icon).unwrap_or("");
    if r.status == 200 && looks_like_icon(name, &r.body) {
        let _ = write_atomic(&out, &r.body);
    }
}

/// The newest release of `repo` (an `owner/name` the catalog validated), from the cache when
/// it is younger than `max_age`. `Err(())`: GitHub could not be reached or read right now.
pub fn latest(
    dir: Option<&Path>,
    repo: &str,
    asset: Option<&str>,
    allow_pre: bool,
    max_age: u64,
) -> Result<Option<Latest>, ()> {
    if !catalog::valid_repo(repo) {
        return Err(());
    }
    let file = dir.map(|d| d.join(UPDATES_FILE));
    let mut all: BTreeMap<String, CacheEntry> = file
        .as_ref()
        .and_then(|f| std::fs::read_to_string(f).ok())
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default();
    let mut e = all.get(repo).cloned().unwrap_or_default();
    let t = now();
    if e.fresh(t, max_age) {
        return if e.ok { Ok(e.latest) } else { Err(()) };
    }
    let path = format!("/repos/{repo}/releases?per_page=10");
    let res = net::get(Host::GitHubApi, &path, e.etag.as_deref(), MAX_RELEASES);
    e.checked = t;
    let out = match res {
        Ok(r) if r.status == 304 && e.ok => Ok(e.latest.clone()),
        Ok(r) if r.status == 200 => {
            match std::str::from_utf8(&r.body)
                .map_err(|e| e.to_string())
                .and_then(|j| updates::pick_latest(j, repo, asset, allow_pre))
            {
                Ok(l) => {
                    e.latest = l.clone();
                    e.etag = r.etag;
                    e.ok = true;
                    Ok(l)
                }
                Err(_) => {
                    e.ok = false;
                    Err(())
                }
            }
        }
        // The repo does not exist (yet), e.g. the Hub's own before its first release.
        Ok(r) if r.status == 404 => {
            e.latest = None;
            e.etag = None;
            e.ok = true;
            Ok(None)
        }
        _ => {
            e.ok = false;
            Err(())
        }
    };
    all.insert(repo.to_string(), e);
    if let Some(f) = file {
        let _ = write_atomic(
            &f,
            serde_json::to_string(&all).unwrap_or_default().as_bytes(),
        );
    }
    out
}

/// The first model of a local Ollama on `127.0.0.1:11434`, if one answers. Nothing else is
/// asked: no other host or port is ever used for questions.
pub fn ollama_model() -> Option<String> {
    let r = net::get(Host::Ollama, "/api/tags", None, 256 * 1024).ok()?;
    if r.status != 200 {
        return None;
    }
    let v: serde_json::Value = serde_json::from_slice(&r.body).ok()?;
    v["models"]
        .as_array()?
        .iter()
        .filter_map(|m| m["name"].as_str())
        .find(|n| !n.is_empty() && n.len() < 200 && !n.contains(['\r', '\n', '"']))
        .map(str::to_string)
}

/// Ask the local model one question. The answer is plain text; the Hub shows it as text and
/// never runs, opens or follows anything in it.
pub fn ollama_ask(model: &str, question: &str) -> Result<String, String> {
    let body = serde_json::json!({
        "model": model,
        "prompt": question.chars().take(4000).collect::<String>(),
        "stream": false,
    })
    .to_string();
    let r = net::post_json(Host::Ollama, "/api/generate", &body, 1024 * 1024, 120_000)?;
    if r.status != 200 {
        return Err(format!("HTTP {}", r.status));
    }
    let v: serde_json::Value = serde_json::from_slice(&r.body).map_err(|e| e.to_string())?;
    let text = v["response"].as_str().ok_or("no answer")?;
    Ok(text.chars().take(20_000).collect())
}
