//! Navigation allowlist for every WebView the Hub owns.
//!
//! The Hub page and each hosted module live on their own virtual origin, served by a custom
//! protocol handler (no local HTTP server). WebView2 shows custom protocols as
//! `https://<scheme>.<host>/`, so the Hub is `https://fastkiwi.hub/` and the InstaWords module
//! is `https://fkmod.instawords/`. A WebView may only navigate inside its own origin; every other
//! URL (other hosts, `http:`, ports, user info, `file:`, `data:`, `javascript:`, `blob:`) is refused.

/// Custom protocol of the Hub page.
pub const HUB_SCHEME: &str = "fastkiwi";
/// Host part of the Hub page.
pub const HUB_HOST: &str = "fastkiwi.hub";
/// Custom protocol of hosted web modules.
pub const MODULE_SCHEME: &str = "fkmod";

/// The origin host of a module: `fkmod.<id>`.
pub fn module_host(app_id: &str) -> String {
    format!("{MODULE_SCHEME}.{app_id}")
}

/// The URL a module WebView starts at (wry shows it as `https://fkmod.<id>/<entry>`).
pub fn module_start_url(app_id: &str, entry: &str) -> String {
    format!("{MODULE_SCHEME}://{app_id}/{entry}")
}

/// The URL the Hub WebView starts at (wry shows it as `https://fastkiwi.hub/index.html`).
pub fn hub_start_url() -> String {
    format!("{HUB_SCHEME}://hub/index.html")
}

/// True when `url` may be shown in a WebView whose origin is `https://<allowed_host>`.
pub fn is_allowed(url: &str, allowed_host: &str) -> bool {
    if url == "about:blank" {
        return true;
    }
    let Ok(u) = url::Url::parse(url) else {
        return false;
    };
    u.scheme() == "https"
        && u.username().is_empty()
        && u.password().is_none()
        && u.port().is_none()
        && u.host_str()
            .is_some_and(|h| h.eq_ignore_ascii_case(allowed_host))
}

/// The host of a URL, for a short, non-identifying note in the UI when a navigation is refused.
pub fn host_of(url: &str) -> String {
    url::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(str::to_string))
        .unwrap_or_else(|| {
            url.split(':')
                .next()
                .unwrap_or("")
                .chars()
                .take(16)
                .collect()
        })
}

/// `<scheme>://<host>/...` with no user info or port (the form wry passes before it rewrites).
fn is_custom_form(url: &str, scheme: &str, host: &str) -> bool {
    url::Url::parse(url).is_ok_and(|u| {
        u.scheme() == scheme
            && u.username().is_empty()
            && u.password().is_none()
            && u.port().is_none()
            && u.host_str().is_some_and(|h| h.eq_ignore_ascii_case(host))
    })
}

/// May the WebView hosting module `app_id` navigate to `url`?
pub fn module_nav_allowed(url: &str, app_id: &str) -> bool {
    crate::catalog::valid_id(app_id)
        && (is_allowed(url, &module_host(app_id)) || is_custom_form(url, MODULE_SCHEME, app_id))
}

/// May the Hub's own WebView navigate to `url`?
pub fn hub_nav_allowed(url: &str) -> bool {
    is_allowed(url, HUB_HOST) || is_custom_form(url, HUB_SCHEME, "hub")
}

/// May the WebView hosting a process module (kind P, `--hub-host`) navigate to `url`? Only
/// `http://127.0.0.1:<port>/…` of the port its own handshake gave, with no user info.
pub fn process_nav_allowed(url: &str, port: u16) -> bool {
    if url == "about:blank" {
        return true;
    }
    let Ok(u) = url::Url::parse(url) else {
        return false;
    };
    u.scheme() == "http"
        && u.username().is_empty()
        && u.password().is_none()
        && u.host_str() == Some("127.0.0.1")
        && u.port() == Some(port)
}

/// The origin prefix web messages from a process module must come from.
pub fn process_origin(port: u16) -> String {
    format!("http://127.0.0.1:{port}/")
}
