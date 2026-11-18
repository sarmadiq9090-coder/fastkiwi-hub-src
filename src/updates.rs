//! Update checks against public GitHub releases: pure parts (parsing, version order, asset
//! choice, cache rules), so they are tested without the network. The fetch itself is
//! `net.rs`; the Hub only reads release metadata and offers the installer link in the browser.

use serde::{Deserialize, Serialize};

/// How long a check result is reused before GitHub is asked again.
pub const AUTO_MAX_AGE_SECS: u64 = 24 * 3600;
/// A "Check now" press reuses a result this recent (rate-limit friendly).
pub const MANUAL_MAX_AGE_SECS: u64 = 10 * 60;
/// After a failure (offline, rate limit), wait this long before an automatic retry.
pub const RETRY_AFTER_FAIL_SECS: u64 = 3600;

/// A version as numbers plus an optional pre-release tag: `v1.8.0`, `1.6.1.0`, `0.3.0-preview`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Version {
    pub nums: Vec<u64>,
    pub pre: Option<String>,
}

impl Version {
    pub fn parse(s: &str) -> Option<Version> {
        let s = s.trim();
        let s = s.strip_prefix(['v', 'V']).unwrap_or(s);
        let s = s.split('+').next()?;
        let (core, pre) = match s.split_once('-') {
            Some((c, p)) if !p.is_empty() => (c, Some(p.to_string())),
            Some(_) => return None,
            None => (s, None),
        };
        let nums: Option<Vec<u64>> = core.split('.').map(|p| p.parse().ok()).collect();
        let nums = nums?;
        (!nums.is_empty() && nums.len() <= 4).then_some(Version { nums, pre })
    }
}

impl PartialOrd for Version {
    fn partial_cmp(&self, o: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(o))
    }
}

impl Ord for Version {
    fn cmp(&self, o: &Self) -> std::cmp::Ordering {
        use std::cmp::Ordering::*;
        let n = self.nums.len().max(o.nums.len());
        for i in 0..n {
            let (a, b) = (
                self.nums.get(i).copied().unwrap_or(0),
                o.nums.get(i).copied().unwrap_or(0),
            );
            if a != b {
                return a.cmp(&b);
            }
        }
        match (&self.pre, &o.pre) {
            (None, None) => Equal,
            (None, Some(_)) => Greater,
            (Some(_), None) => Less,
            (Some(a), Some(b)) => a.cmp(b),
        }
    }
}

/// `*` matches any text; everything else matches itself, ignoring case.
pub fn glob(pattern: &str, name: &str) -> bool {
    let (p, n) = (pattern.to_ascii_lowercase(), name.to_ascii_lowercase());
    let parts: Vec<&str> = p.split('*').collect();
    if parts.len() == 1 {
        return p == n;
    }
    let mut rest = n.as_str();
    for (i, part) in parts.iter().enumerate() {
        if i == 0 {
            let Some(r) = rest.strip_prefix(part) else {
                return false;
            };
            rest = r;
        } else if i == parts.len() - 1 {
            return rest.ends_with(part);
        } else if let Some(at) = rest.find(part) {
            rest = &rest[at + part.len()..];
        } else {
            return false;
        }
    }
    true
}

/// The newest release of a repo, as far as the Hub cares.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Latest {
    /// The version, as the tag says it (without a leading `v`).
    pub version: String,
    /// The installer to offer, on `https://github.com/<repo>/releases/download/`.
    pub download: Option<String>,
    /// The release page.
    pub page: String,
    pub prerelease: bool,
}

#[derive(Deserialize)]
struct GhAsset {
    name: String,
    browser_download_url: String,
}

#[derive(Deserialize)]
struct GhRelease {
    tag_name: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
    #[serde(default)]
    html_url: String,
    #[serde(default)]
    assets: Vec<GhAsset>,
}

/// Pick the newest release from `GET /repos/<repo>/releases` (a JSON array). Drafts are
/// skipped; pre-releases only when `allow_pre`. `Ok(None)`: the repo has no such release.
pub fn pick_latest(
    json: &str,
    repo: &str,
    asset: Option<&str>,
    allow_pre: bool,
) -> Result<Option<Latest>, String> {
    let list: Vec<GhRelease> = serde_json::from_str(json).map_err(|e| e.to_string())?;
    let best = list
        .iter()
        .filter(|r| !r.draft && (allow_pre || !r.prerelease))
        .filter_map(|r| Version::parse(&r.tag_name).map(|v| (v, r)))
        .max_by(|a, b| a.0.cmp(&b.0));
    let Some((_, r)) = best else {
        return Ok(None);
    };
    let prefix = crate::links::release_asset_prefix(repo);
    let download = asset.and_then(|pat| {
        r.assets
            .iter()
            .find(|a| glob(pat, &a.name) && a.browser_download_url.starts_with(&prefix))
            .map(|a| a.browser_download_url.clone())
            .filter(|u| crate::links::plain_https(u).is_some())
    });
    let page = if r.html_url.starts_with(&format!("https://github.com/{repo}/releases/"))
        && crate::links::plain_https(&r.html_url).is_some()
    {
        r.html_url.clone()
    } else {
        crate::links::releases_page(repo)
    };
    let tag = r.tag_name.trim();
    Ok(Some(Latest {
        version: tag.strip_prefix(['v', 'V']).unwrap_or(tag).to_string(),
        download,
        page,
        prerelease: r.prerelease,
    }))
}

/// What one check found, as the Updates screen shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "kebab-case")]
pub enum Check {
    /// A newer release than the installed copy.
    Available { latest: Latest },
    /// The installed copy is the newest (or newer).
    Current { latest: Latest },
    /// The installed version is not known; this is the newest release.
    Unknown { latest: Latest },
    /// The repo has no release yet (404 or an empty list).
    NoRelease,
    /// Offline, rate-limited, or an answer the Hub could not read.
    Unreachable,
}

pub fn compare(installed: Option<&str>, latest: Option<Latest>) -> Check {
    let Some(latest) = latest else {
        return Check::NoRelease;
    };
    let (Some(have), Some(new)) = (
        installed.and_then(Version::parse),
        Version::parse(&latest.version),
    ) else {
        return Check::Unknown { latest };
    };
    if new > have {
        Check::Available { latest }
    } else {
        Check::Current { latest }
    }
}

/// One repo's cached answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct CacheEntry {
    /// Seconds since 1970 of the last answer (or failure).
    pub checked: u64,
    #[serde(default)]
    pub etag: Option<String>,
    /// The last good answer (`None` with `ok`: no release).
    #[serde(default)]
    pub latest: Option<Latest>,
    /// The last request got an answer (200, 304 or 404).
    #[serde(default)]
    pub ok: bool,
}

impl CacheEntry {
    /// Use the cache instead of asking GitHub.
    pub fn fresh(&self, now: u64, max_age: u64) -> bool {
        let age = now.saturating_sub(self.checked);
        if self.ok {
            age < max_age
        } else {
            age < RETRY_AFTER_FAIL_SECS.min(max_age)
        }
    }
}
