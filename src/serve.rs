//! What the custom protocol handlers serve: the embedded Hub UI, and a module's files from its
//! local folder (or the embedded stub module). Every response carries a Content-Security-Policy
//! that allows only the WebView's own origin, so neither the Hub nor a module can reach the
//! network (`connect-src 'self'`), frame other pages or start workers.

use std::path::{Component, Path, PathBuf};

include!(concat!(env!("OUT_DIR"), "/ui_files.rs"));

/// CSP for the Hub page.
pub const HUB_CSP: &str = "default-src 'none'; script-src 'self'; style-src 'self' 'unsafe-inline'; \
img-src 'self' data:; font-src 'self'; connect-src 'self'; base-uri 'none'; form-action 'none'; \
frame-src 'none'; frame-ancestors 'none'; object-src 'none'; worker-src 'none'";

/// CSP for a hosted module: its own origin only. Inline styles are allowed (many web builds use
/// them); inline scripts, eval, workers, frames and every other origin are not.
pub const MODULE_CSP: &str = "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; \
img-src 'self' data:; font-src 'self'; connect-src 'self'; media-src 'self'; base-uri 'self'; \
form-action 'none'; frame-src 'none'; frame-ancestors 'none'; object-src 'none'; worker-src 'none'; \
manifest-src 'none'";

/// Media type by extension. Unknown types are served as bytes the WebView will not run.
pub fn mime(path: &str) -> &'static str {
    let ext = path
        .rsplit_once('.')
        .map(|(_, e)| e.to_ascii_lowercase())
        .unwrap_or_default();
    match ext.as_str() {
        "html" | "htm" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" => "application/json",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "ico" => "image/x-icon",
        "webp" => "image/webp",
        "woff2" => "font/woff2",
        "woff" => "font/woff",
        "txt" => "text/plain; charset=utf-8",
        "webmanifest" => "application/manifest+json",
        _ => "application/octet-stream",
    }
}

fn percent_decode(s: &str) -> Option<String> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' {
            let hex = s.get(i + 1..i + 3)?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// Turn a request path (`/assets/app.js?x=1`) into a safe relative path (`assets/app.js`).
/// Refuses `..`, `.`, backslashes, drive letters, streams, NUL and empty segments.
/// An empty path gives `default`.
pub fn clean_rel_path(url_path: &str, default: &str) -> Option<String> {
    let path = url_path.split(['?', '#']).next().unwrap_or("");
    let decoded = percent_decode(path)?;
    let trimmed = decoded.trim_start_matches('/');
    if trimmed.is_empty() {
        return Some(default.to_string());
    }
    if trimmed.contains(['\\', ':', '\0', '*', '?', '"', '<', '>', '|']) {
        return None;
    }
    let mut segs = Vec::new();
    for seg in trimmed.split('/') {
        if seg.is_empty() || seg == "." || seg == ".." || seg.ends_with('.') || seg.ends_with(' ') {
            return None;
        }
        segs.push(seg);
    }
    let joined = segs.join("/");
    // A folder request gets its index page.
    if joined.ends_with('/') {
        return None;
    }
    Some(joined)
}

/// An embedded UI file by its path under `ui/`.
pub fn ui_file(rel: &str) -> Option<&'static [u8]> {
    UI_FILES.iter().find(|(p, _)| *p == rel).map(|(_, b)| *b)
}

/// The embedded stub module's files live under `ui/stub-module/`.
pub fn stub_file(rel: &str) -> Option<&'static [u8]> {
    // The stub reuses the family fonts and icons from the Hub's own tree.
    let r = rel.strip_prefix("hub-assets/");
    match r {
        Some(shared) if shared.starts_with("fonts/") || shared.starts_with("css/base.css") => {
            ui_file(shared)
        }
        _ => ui_file(&format!("stub-module/{rel}")),
    }
}

/// Resolve `rel` inside `root` on disk, refusing anything that escapes it (`..`, links or
/// junctions that point elsewhere). Returns the file path when it is a regular file.
pub fn resolve_in_root(root: &Path, rel: &str) -> Option<PathBuf> {
    let root = std::fs::canonicalize(root).ok()?;
    let mut p = root.clone();
    for c in Path::new(rel).components() {
        match c {
            Component::Normal(s) => p.push(s),
            _ => return None,
        }
    }
    let real = std::fs::canonicalize(&p).ok()?;
    (real.starts_with(&root) && real.is_file()).then_some(real)
}
