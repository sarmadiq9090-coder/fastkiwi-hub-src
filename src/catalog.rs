//! The app catalog: the site's `data/apps.json` (schema 1), read by the Hub as untrusted data.
//!
//! One entry per app is the whole job of adding a program (docs/ADDING-AN-APP.md). The Hub
//! fetches `https://fastkiwi.app/data/apps.json`, keeps the last good copy on disk, and falls
//! back to the copy compiled into the exe (`catalog/apps.json`). Every field the site already
//! has is used as is; the Hub adds a few optional ones (`platform`, `download`, `release`,
//! `hub`). Unknown fields are ignored, so the site can grow without breaking older Hubs.
//!
//! Validation is strict where it matters for safety: ids, every URL the Hub may open (only
//! `https://fastkiwi.app/`, `https://github.com/sarmadiq9090-coder/`, `https://apps.microsoft.com/`),
//! icons, release repos, and every detection probe (paths only under the user's or program
//! folders, plain `.exe` names). One bad value makes the whole catalog invalid, and the Hub
//! keeps the copy it had. Nothing in the catalog is ever run: the Hub launches only an exe
//! that detection found on this PC.
//!
//! Visibility (council verdict 4, D-069): only `released` apps are shown. Other known statuses
//! (`coming-soon`) are kept only when the dev flag is on.

use serde::{Deserialize, Serialize};

/// The catalog compiled into the exe: first-run data and the offline fallback.
pub const FIXTURE: &str = include_str!("../catalog/apps.json");

/// Text in the two UI languages (BCP 47 `ar`, `en`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Text {
    pub ar: String,
    pub en: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Listing {
    pub id: String,
    pub live: bool,
}

/// Where the app runs. The Hub detects and launches only Windows apps; phone apps are shown
/// with a link to their page.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Platform {
    #[default]
    Windows,
    Android,
}

/// How an app appears in the Hub (HUB-DESIGN §2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Kind {
    /// Static web module, mapped from the installed app's `web/` folder.
    S,
    /// Process module (`--hub-host`).
    P,
    /// Launch only.
    #[default]
    L,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Installer {
    Nsis,
    Inno,
}

/// One read-only way to find an installed copy (HUB-DESIGN §4, "Detection per channel").
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Probe {
    /// `PackageManager.FindPackagesForUser("", pfn)`; launched as `<pfn>!<app_id>`.
    Store { pfn: String, app_id: String },
    /// An uninstall key under HKCU / HKLM / HKLM (32-bit view).
    Uninstall {
        key: String,
        installer: Installer,
        exe: String,
    },
    /// winget `Links\<link>` and `Packages\<package_id>_*\<exe>`.
    Winget {
        package_id: String,
        link: String,
        exe: String,
    },
    /// A portable copy at a known place (`%VAR%\...`).
    Path { path: String },
    /// The exe named by `HKCU\...\CurrentVersion\Run` value `name`.
    RunValue { name: String },
    /// A key whose presence only says "used on this PC"; never an install by itself.
    RegHint { key: String },
    /// A probe kind this Hub does not know (from a newer catalog): skipped.
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Module {
    /// Folder inside the install that holds the web build (kind S).
    pub web_dir: String,
    /// Entry page inside `web_dir`.
    pub entry: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OnDemand {
    pub what: Text,
    pub size_mb: u32,
}

/// The optional Windows block: how to find, open and host the app.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Hub {
    #[serde(default)]
    pub kind: Kind,
    /// Download size, shown on the card.
    #[serde(default)]
    pub size_mb: Option<u32>,
    #[serde(default)]
    pub on_demand: Vec<OnDemand>,
    #[serde(default)]
    pub needs: Option<Text>,
    #[serde(default)]
    pub data_dir: Option<String>,
    #[serde(default)]
    pub module: Option<Module>,
    /// Detection probes, in order. Optional: without them the app is shown as "not installed"
    /// with its Download button.
    #[serde(default)]
    pub detect: Vec<Probe>,
}

/// Where the app's releases are, for update checks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Release {
    /// `sarmadiq9090-coder/<repo>` on GitHub.
    pub repo: String,
    /// The installer asset in a release: a file name, `*` matches any text
    /// (`InstaWords_*_x64-setup.exe`). Without it, the release page is offered.
    #[serde(default)]
    pub asset: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct App {
    pub id: String,
    pub status: String,
    pub name: Text,
    pub tagline: Text,
    #[serde(default)]
    pub description: Option<Text>,
    /// The app's page on fastkiwi.app, per language.
    #[serde(default)]
    pub url: Option<Text>,
    #[serde(default)]
    pub store: Option<Listing>,
    #[serde(default)]
    pub winget: Option<Listing>,
    /// `img/<name>.svg|png` (relative to fastkiwi.app, as the site writes it), or an
    /// `https://fastkiwi.app/...` URL. Missing or unknown: a generic icon.
    #[serde(default)]
    pub icon: String,
    #[serde(default)]
    pub platform: Platform,
    /// The direct installer URL (Windows apps).
    #[serde(default)]
    pub download: Option<String>,
    #[serde(default)]
    pub release: Option<Release>,
    #[serde(default)]
    pub hub: Option<Hub>,
    /// Who makes the app. Missing or `FastKiwi`: a FastKiwi app. Anything else is a
    /// third-party program, labelled «ليس من FastKiwi» / "Not by FastKiwi".
    #[serde(default)]
    pub publisher: Option<String>,
}

impl App {
    /// A program not made by FastKiwi.
    pub fn third_party(&self) -> bool {
        self.publisher
            .as_deref()
            .is_some_and(|p| !p.trim().eq_ignore_ascii_case("fastkiwi"))
    }

    /// May the Hub open `url` for this entry?
    pub fn may_open(&self, url: &str) -> bool {
        if self.third_party() {
            crate::links::allowed_third_party_url(url)
        } else {
            crate::links::allowed_open_url(url)
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Catalog {
    pub schema: u32,
    /// Grows with every change; the Hub never accepts a lower serial than it has seen.
    pub serial: u64,
    /// `YYYY-MM-DD`: a fetched catalog past this day is refused.
    pub expires: String,
    pub apps: Vec<App>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CatalogError {
    Json(String),
    Schema(u32),
    Serial,
    Expires(String),
    DuplicateId(String),
    BadId(String),
    BadIcon(String),
    BadUrl { app: String, url: String },
    BadRelease { app: String, why: String },
    BadProbe { app: String, why: String },
    /// Fetched catalogs only: past its `expires` day.
    Expired(String),
    /// Fetched catalogs only: an older catalog than one already seen.
    OldSerial { got: u64, have: u64 },
    /// Fetched catalogs only: no good signature by a pinned key.
    Signature(String),
}

impl std::fmt::Display for CatalogError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Json(e) => write!(f, "catalog is not valid JSON: {e}"),
            Self::Schema(s) => write!(f, "catalog schema {s} is not supported (want 1)"),
            Self::Serial => write!(f, "catalog serial must be at least 1"),
            Self::Expires(s) => write!(f, "catalog expiry {s:?} is not a YYYY-MM-DD date"),
            Self::DuplicateId(id) => write!(f, "app id {id:?} appears twice"),
            Self::BadId(id) => write!(f, "app id {id:?} is not [a-z0-9-]{{1,32}}"),
            Self::BadIcon(i) => write!(f, "icon {i:?} is not img/<name>.svg|png on fastkiwi.app"),
            Self::BadUrl { app, url } => write!(f, "app {app:?}: URL {url:?} is not allowed"),
            Self::BadRelease { app, why } => write!(f, "app {app:?}: release {why}"),
            Self::BadProbe { app, why } => write!(f, "app {app:?}: {why}"),
            Self::Expired(d) => write!(f, "catalog expired on {d}"),
            Self::OldSerial { got, have } => {
                write!(f, "catalog serial {got} is older than {have}")
            }
            Self::Signature(e) => write!(f, "catalog signature: {e}"),
        }
    }
}

impl std::error::Error for CatalogError {}

/// Statuses the Hub knows. Anything else is dropped.
const KNOWN_STATUS: [&str; 2] = ["released", "coming-soon"];

/// The GitHub account whose releases and pages the Hub may open.
pub const GITHUB_OWNER: &str = "sarmadiq9090-coder";

/// Environment variables a catalog path may start with. Nothing else: no drive letters, no UNC.
pub const PATH_ROOTS: [&str; 5] = [
    "%LOCALAPPDATA%",
    "%APPDATA%",
    "%USERPROFILE%",
    "%PROGRAMFILES%",
    "%PROGRAMFILES(X86)%",
];

/// Where a `path` probe may point: the fixed install places only (council C2).
pub const LAUNCH_PATH_ROOTS: [&str; 3] = [
    "%PROGRAMFILES%\\",
    "%PROGRAMFILES(X86)%\\",
    "%LOCALAPPDATA%\\Programs\\",
];

/// Registry roots a `reg_hint` key may use.
const HINT_ROOTS: [&str; 1] = ["HKCU\\Software\\"];

pub fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 32
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

/// A plain file name ending in `.exe`: no separators, no `..`, no drive or stream.
pub fn valid_exe_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    !name.is_empty()
        && name.len() <= 64
        && lower.ends_with(".exe")
        && !name.contains(['\\', '/', ':', '*', '?', '"', '<', '>', '|'])
        && !name.contains("..")
        && name.trim() == name
}

/// `Name_PublisherId`: what `Package.Id.FamilyName` looks like.
pub fn valid_pfn(pfn: &str) -> bool {
    let Some((name, publisher)) = pfn.rsplit_once('_') else {
        return false;
    };
    (3..=50).contains(&name.len())
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-')
        && publisher.len() == 13
        && publisher
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
}

fn valid_app_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'.')
}

/// A Microsoft Store product id (`9NGB1F7T7JGN`).
pub fn valid_store_id(id: &str) -> bool {
    (10..=16).contains(&id.len())
        && id
            .bytes()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
}

/// A catalog path: `%ROOT%\` + relative segments, no `..`, no `:` after the root.
pub fn valid_catalog_path(p: &str) -> bool {
    let Some(rest) = PATH_ROOTS.iter().find_map(|r| {
        p.get(..r.len())
            .filter(|h| h.eq_ignore_ascii_case(r))
            .map(|_| &p[r.len()..])
    }) else {
        return false;
    };
    let Some(rest) = rest.strip_prefix('\\') else {
        return false;
    };
    !rest.is_empty()
        && !rest.contains(['/', ':', '%', '*', '?', '"', '<', '>', '|'])
        && rest
            .split('\\')
            .all(|seg| !seg.is_empty() && seg != "." && seg != "..")
}

/// An uninstall key name: a GUID-ish token, `{GUID}_is1`, or a reverse-DNS app id.
fn valid_uninstall_key(k: &str) -> bool {
    !k.is_empty()
        && k.len() <= 80
        && k.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"{}-_.".contains(&b))
}

fn valid_package_id(id: &str) -> bool {
    id.contains('.')
        && id.len() <= 100
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-')
}

fn valid_value_name(n: &str) -> bool {
    !n.is_empty() && n.len() <= 64 && n.bytes().all(|b| b.is_ascii_alphanumeric())
}

fn valid_hint_key(k: &str) -> bool {
    HINT_ROOTS.iter().any(|r| {
        k.get(..r.len()).is_some_and(|h| h.eq_ignore_ascii_case(r))
            && k[r.len()..]
                .split('\\')
                .all(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_alphanumeric()))
    })
}

fn valid_web_dir(d: &str) -> bool {
    !d.is_empty()
        && d.len() <= 32
        && d.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

fn valid_entry(e: &str) -> bool {
    e.ends_with(".html")
        && e.len() <= 64
        && e.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
        && !e.contains("..")
}

/// A plain file name for an icon: `[A-Za-z0-9._-]`, ending in `.svg` or `.png`.
fn valid_icon_name(n: &str) -> bool {
    let l = n.to_ascii_lowercase();
    (l.ends_with(".svg") || l.ends_with(".png"))
        && n.len() <= 64
        && !n.contains("..")
        && n.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
}

/// An icon value: empty, `img/<name>`, `icons/<name>`, or `https://fastkiwi.app/<path>/<name>`.
pub fn valid_icon(icon: &str) -> bool {
    if icon.is_empty() {
        return true;
    }
    if let Some(n) = icon
        .strip_prefix("img/")
        .or_else(|| icon.strip_prefix("icons/"))
    {
        return valid_icon_name(n);
    }
    crate::links::site_url(icon).is_some_and(|u| {
        u.query().is_none()
            && u.path_segments()
                .and_then(|mut s| s.next_back())
                .is_some_and(valid_icon_name)
    })
}

/// A release asset pattern: a plain file name, where `*` matches any text.
fn valid_asset(a: &str) -> bool {
    !a.is_empty()
        && a.len() <= 80
        && a.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_.*".contains(&b))
        && !a.contains("..")
}

/// `<owner>/<repo>` on GitHub, made of plain characters.
pub fn valid_repo(r: &str) -> bool {
    let plain = |x: &str| {
        !x.is_empty()
            && x.len() <= 100
            && x != "."
            && x != ".."
            && x.bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
    };
    r.split_once('/')
        .is_some_and(|(owner, name)| plain(owner) && plain(name))
}

/// FastKiwi's own repos: `sarmadiq9090-coder/<repo>`.
pub fn own_repo(r: &str) -> bool {
    valid_repo(r) && r.split_once('/').is_some_and(|(owner, _)| owner == GITHUB_OWNER)
}

fn check_probe(p: &Probe) -> Result<(), String> {
    match p {
        Probe::Store { pfn, app_id } => {
            if !valid_pfn(pfn) {
                return Err(format!("store pfn {pfn:?} is not Name_publisherid"));
            }
            if !valid_app_id(app_id) {
                return Err(format!("store app_id {app_id:?} is not valid"));
            }
        }
        Probe::Uninstall { key, exe, .. } => {
            if !valid_uninstall_key(key) {
                return Err(format!("uninstall key {key:?} is not valid"));
            }
            if !valid_exe_name(exe) {
                return Err(format!("exe {exe:?} is not a plain .exe name"));
            }
        }
        Probe::Winget {
            package_id,
            link,
            exe,
        } => {
            if !valid_package_id(package_id) {
                return Err(format!("winget id {package_id:?} is not valid"));
            }
            if !valid_exe_name(link) || !valid_exe_name(exe) {
                return Err("winget link/exe must be plain .exe names".into());
            }
        }
        Probe::Path { path } => {
            let fixed = LAUNCH_PATH_ROOTS.iter().any(|r| {
                path.get(..r.len()).is_some_and(|h| h.eq_ignore_ascii_case(r))
            });
            if !fixed
                || !valid_catalog_path(path)
                || !valid_exe_name(path.rsplit('\\').next().unwrap_or(""))
            {
                return Err(format!(
                    "path {path:?} must be in Program Files or %LOCALAPPDATA%\\Programs, ending in name.exe"
                ));
            }
        }
        Probe::RunValue { name } => {
            if !valid_value_name(name) {
                return Err(format!("run value {name:?} is not valid"));
            }
        }
        Probe::RegHint { key } => {
            if !valid_hint_key(key) {
                return Err(format!("hint key {key:?} must be under HKCU\\Software\\"));
            }
        }
        Probe::Unknown => {}
    }
    Ok(())
}

pub fn valid_date(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 10
        && b[4] == b'-'
        && b[7] == b'-'
        && b.iter()
            .enumerate()
            .all(|(i, c)| i == 4 || i == 7 || c.is_ascii_digit())
}

fn check_app(app: &App) -> Result<(), CatalogError> {
    let bad_url = |url: &str| CatalogError::BadUrl {
        app: app.id.clone(),
        url: url.to_string(),
    };
    if let Some(p) = &app.publisher
        && (p.trim().is_empty() || p.len() > 80 || p.chars().any(char::is_control))
    {
        return Err(CatalogError::BadRelease {
            app: app.id.clone(),
            why: "publisher must be a short name".into(),
        });
    }
    if !valid_icon(&app.icon) {
        return Err(CatalogError::BadIcon(app.icon.clone()));
    }
    if let Some(u) = &app.url {
        for x in [&u.ar, &u.en] {
            if !app.may_open(x) {
                return Err(bad_url(x));
            }
        }
    }
    if let Some(d) = &app.download
        && !app.may_open(d)
    {
        return Err(bad_url(d));
    }
    if let Some(s) = &app.store
        && !valid_store_id(&s.id)
    {
        return Err(CatalogError::BadRelease {
            app: app.id.clone(),
            why: format!("store id {:?} is not a product id", s.id),
        });
    }
    if let Some(r) = &app.release {
        let bad = |why: String| CatalogError::BadRelease {
            app: app.id.clone(),
            why,
        };
        let ok = if app.third_party() {
            valid_repo(&r.repo)
        } else {
            own_repo(&r.repo)
        };
        if !ok {
            return Err(bad(format!("repo {:?} is not an allowed owner/name", r.repo)));
        }
        if let Some(a) = &r.asset
            && !valid_asset(a)
        {
            return Err(bad(format!("asset {a:?} is not a plain file name")));
        }
    }
    if let Some(hub) = &app.hub {
        let bad = |why: String| CatalogError::BadProbe {
            app: app.id.clone(),
            why,
        };
        if let Some(d) = &hub.data_dir
            && !valid_catalog_path(d)
        {
            return Err(bad(format!("data_dir {d:?} is not %ROOT%\\...")));
        }
        for p in &hub.detect {
            check_probe(p).map_err(bad)?;
        }
        if let Some(m) = &hub.module {
            if hub.kind != Kind::S {
                return Err(bad("only kind S apps may have a web module".into()));
            }
            if !valid_web_dir(&m.web_dir) || !valid_entry(&m.entry) {
                return Err(bad("module web_dir/entry are not plain names".into()));
            }
        }
    }
    Ok(())
}

/// Parse and validate a catalog. Returns every app, whatever its status; use [`visible`] to filter.
pub fn parse(json: &str) -> Result<Catalog, CatalogError> {
    let cat: Catalog = serde_json::from_str(json).map_err(|e| CatalogError::Json(e.to_string()))?;
    if cat.schema != 1 {
        return Err(CatalogError::Schema(cat.schema));
    }
    if cat.serial == 0 {
        return Err(CatalogError::Serial);
    }
    if !valid_date(&cat.expires) {
        return Err(CatalogError::Expires(cat.expires.clone()));
    }
    let mut seen = std::collections::BTreeSet::new();
    for app in &cat.apps {
        if !valid_id(&app.id) || app.id.starts_with("my-") {
            return Err(CatalogError::BadId(app.id.clone()));
        }
        if !seen.insert(app.id.clone()) {
            return Err(CatalogError::DuplicateId(app.id.clone()));
        }
        check_app(app)?;
    }
    Ok(cat)
}

/// A fetched (or cached) catalog: valid, not past `expires` on `today` (`YYYY-MM-DD`), and not
/// older than `min_serial`.
pub fn accept(json: &str, today: &str, min_serial: u64) -> Result<Catalog, CatalogError> {
    let c = parse(json)?;
    if today > c.expires.as_str() {
        return Err(CatalogError::Expired(c.expires));
    }
    if c.serial < min_serial {
        return Err(CatalogError::OldSerial {
            got: c.serial,
            have: min_serial,
        });
    }
    Ok(c)
}

/// Largest catalog accepted from the site or the cache.
pub const MAX_BYTES: usize = 512 * 1024;

/// A fetched or cached catalog: a good signature by a pinned key (`trust.rs`) over these exact
/// bytes, then [`accept`]. `min_serial` is the highest serial this PC has seen (rollback
/// protection).
pub fn accept_signed(
    json: &str,
    sig: &str,
    today: &str,
    min_serial: u64,
) -> Result<Catalog, CatalogError> {
    if json.len() > MAX_BYTES {
        return Err(CatalogError::Json("catalog too large".into()));
    }
    let c = accept(json, today, min_serial)?;
    crate::trust::verify(json.as_bytes(), sig, c.serial).map_err(CatalogError::Signature)?;
    Ok(c)
}

/// The catalog to start with: the cached copy from the site when its signature is good, it
/// has not expired and it is not older than the copy inside the exe or the highest serial
/// seen; else the copy inside the exe. `true`: the cached copy.
pub fn choose(
    compiled: &str,
    cached: Option<(&str, &str)>,
    today: &str,
    seen_serial: u64,
) -> (Catalog, bool) {
    let base = parse(compiled).unwrap_or(Catalog {
        schema: 1,
        serial: 1,
        expires: "2000-01-01".into(),
        apps: Vec::new(),
    });
    let min = base.serial.max(seen_serial);
    match cached.and_then(|(t, sig)| accept_signed(t, sig, today, min).ok()) {
        Some(c) => (c, true),
        None => (base, false),
    }
}

/// The apps the Hub shows: a known status, and `released` unless `dev` is on.
pub fn visible(cat: &Catalog, dev: bool) -> Vec<App> {
    cat.apps
        .iter()
        .filter(|a| KNOWN_STATUS.contains(&a.status.as_str()))
        .filter(|a| dev || a.status == "released")
        .cloned()
        .collect()
}

/// Today's date (UTC) as `YYYY-MM-DD`, from seconds since 1970.
pub fn date_from_unix(secs: u64) -> String {
    // Howard Hinnant's civil_from_days.
    let z = (secs / 86_400) as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
}
