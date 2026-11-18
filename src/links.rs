//! Which URLs the Hub may open in the default browser, and where they come from.
//!
//! The Hub page never hands the exe a URL: it names an app and what to open (`download`,
//! `page`, `store`, `update`), and the exe looks the URL up in the validated catalog or in the
//! last update check. Opening is ShellExecute of an `https:` URL, so the browser shows it and
//! the person decides; the Hub itself never downloads or runs anything from the internet.
//!
//! FastKiwi's own entries may only point at `https://fastkiwi.app/`, FastKiwi's GitHub account
//! and `https://apps.microsoft.com/`. A third-party entry (its `publisher` is not FastKiwi) may
//! point at any plain `https:` URL; the Hub labels it «not by FastKiwi».

use url::Url;

pub const SITE: &str = "https://fastkiwi.app/";
pub const SUPPORT: &str = "https://fastkiwi.app/support/";
pub const HUB_REPO: &str = "sarmadiq9090-coder/fastkiwi-hub";
pub const HUB_RELEASES: &str = "https://github.com/sarmadiq9090-coder/fastkiwi-hub/releases";
pub const CATALOG_URL: &str = "https://fastkiwi.app/data/apps.json";
/// Microsoft's Evergreen WebView2 Runtime bootstrapper (official link).
pub const WEBVIEW2_URL: &str = "https://go.microsoft.com/fwlink/p/?LinkId=2124703";

/// A plain `https:` URL: no user info, no port, a real host name, at most 2 KB.
pub fn plain_https(s: &str) -> Option<Url> {
    if s.len() > 2048 || s.chars().any(|c| c.is_control() || c.is_whitespace()) {
        return None;
    }
    let u = Url::parse(s).ok()?;
    let host = u.host_str()?;
    (u.scheme() == "https"
        && u.username().is_empty()
        && u.password().is_none()
        && u.port().is_none()
        && host.contains('.')
        && u.host().is_some_and(|h| matches!(h, url::Host::Domain(_))))
    .then_some(u)
}

/// A URL on fastkiwi.app.
pub fn site_url(s: &str) -> Option<Url> {
    plain_https(s).filter(|u| u.host_str() == Some("fastkiwi.app"))
}

/// A URL a FastKiwi entry may open.
pub fn allowed_open_url(s: &str) -> bool {
    let Some(u) = plain_https(s) else {
        return false;
    };
    match u.host_str() {
        Some("fastkiwi.app") | Some("apps.microsoft.com") => true,
        Some("github.com") => u
            .path()
            .starts_with(&format!("/{}/", crate::catalog::GITHUB_OWNER)),
        _ => false,
    }
}

/// A URL a third-party entry may open: any plain https URL.
pub fn allowed_third_party_url(s: &str) -> bool {
    plain_https(s).is_some()
}

/// The Store page of a product, in the browser (it offers to open the Store app).
pub fn store_page(id: &str) -> Option<String> {
    crate::catalog::valid_store_id(id).then(|| format!("https://apps.microsoft.com/detail/{id}"))
}

/// Where a release asset of `repo` may be downloaded from.
pub fn release_asset_prefix(repo: &str) -> String {
    format!("https://github.com/{repo}/releases/download/")
}

/// The releases page of `repo`.
pub fn releases_page(repo: &str) -> String {
    format!("https://github.com/{repo}/releases")
}

/// The remote icon of a catalog entry, if it is not one of the icons inside the exe:
/// `img/x.svg` is relative to fastkiwi.app, as the site writes it.
pub fn remote_icon(icon: &str) -> Option<Url> {
    if let Some(rest) = icon.strip_prefix("img/") {
        return site_url(&format!("{SITE}img/{rest}"));
    }
    site_url(icon)
}

/// The file name at the end of an icon value (`img/lemon.png` → `lemon.png`).
pub fn icon_file_name(icon: &str) -> Option<&str> {
    let n = icon.rsplit('/').next()?;
    (!n.is_empty()).then_some(n)
}
