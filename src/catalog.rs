//! The fixture catalog (`catalog/apps.json`): the site's apps.json schema 1 plus a draft `hub`
//! block per app. Parsing is strict: an unknown schema, a bad id, a probe that points outside the
//! allowed places, or an unknown field inside `hub` makes the whole file invalid, so a damaged
//! catalog can never make the Hub read or launch something unexpected.
//!
//! Visibility (council verdict 4, D-069): only `released` apps are shown. Other known statuses
//! (`coming-soon`) are kept only when the dev fixture flag is on.

use serde::{Deserialize, Serialize};

/// The catalog compiled into the exe.
pub const FIXTURE: &str = include_str!("../catalog/apps.json");
/// The Updates screen fixture compiled into the exe (UI only).
pub const UPDATES_FIXTURE: &str = include_str!("../catalog/updates.fixture.json");

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

/// How an app appears in the Hub (HUB-DESIGN §2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Kind {
    /// Static web module, mapped from the installed app's `web/` folder.
    S,
    /// Process module (`--hub-host`, Phase D). Launch only until then.
    P,
    /// Launch only.
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
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
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
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Module {
    /// Folder inside the install that holds the web build (kind S).
    pub web_dir: String,
    /// Entry page inside `web_dir`.
    pub entry: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OnDemand {
    pub what: Text,
    pub size_mb: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Hub {
    pub kind: Kind,
    pub size_mb: u32,
    #[serde(default)]
    pub on_demand: Vec<OnDemand>,
    #[serde(default)]
    pub needs: Option<Text>,
    pub data_dir: String,
    #[serde(default)]
    pub module: Option<Module>,
    pub detect: Vec<Probe>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct App {
    pub id: String,
    pub status: String,
    pub name: Text,
    pub tagline: Text,
    #[serde(default)]
    pub store: Option<Listing>,
    #[serde(default)]
    pub winget: Option<Listing>,
    pub icon: String,
    /// Apps without a `hub` block are not shown in the Hub.
    #[serde(default)]
    pub hub: Option<Hub>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Catalog {
    pub schema: u32,
    pub serial: u64,
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
    BadProbe { app: String, why: String },
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
            Self::BadIcon(i) => write!(f, "icon {i:?} is not a plain icons/<name>.svg|png"),
            Self::BadProbe { app, why } => write!(f, "app {app:?}: {why}"),
        }
    }
}

impl std::error::Error for CatalogError {}

/// Statuses the Hub knows. Anything else is dropped.
const KNOWN_STATUS: [&str; 2] = ["released", "coming-soon"];

/// Environment variables a catalog path may start with. Nothing else: no drive letters, no UNC.
pub const PATH_ROOTS: [&str; 5] = [
    "%LOCALAPPDATA%",
    "%APPDATA%",
    "%USERPROFILE%",
    "%PROGRAMFILES%",
    "%PROGRAMFILES(X86)%",
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
            if !valid_catalog_path(path) || !valid_exe_name(path.rsplit('\\').next().unwrap_or(""))
            {
                return Err(format!("path {path:?} must be %ROOT%\\...\\name.exe"));
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
    }
    Ok(())
}

fn valid_date(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 10
        && b[4] == b'-'
        && b[7] == b'-'
        && b.iter()
            .enumerate()
            .all(|(i, c)| i == 4 || i == 7 || c.is_ascii_digit())
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
        if !valid_id(&app.id) {
            return Err(CatalogError::BadId(app.id.clone()));
        }
        if !seen.insert(app.id.clone()) {
            return Err(CatalogError::DuplicateId(app.id.clone()));
        }
        let icon_ok = app.icon.strip_prefix("icons/").is_some_and(|n| {
            (n.ends_with(".svg") || n.ends_with(".png"))
                && n.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
                && !n.contains("..")
        });
        if !icon_ok {
            return Err(CatalogError::BadIcon(app.icon.clone()));
        }
        if let Some(hub) = &app.hub {
            let bad = |why: String| CatalogError::BadProbe {
                app: app.id.clone(),
                why,
            };
            if !valid_catalog_path(&hub.data_dir) {
                return Err(bad(format!(
                    "data_dir {:?} is not %ROOT%\\...",
                    hub.data_dir
                )));
            }
            if hub.detect.is_empty() {
                return Err(bad("no detect probes".into()));
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
    }
    Ok(cat)
}

/// The apps the Hub shows: a `hub` block, a known status, and `released` unless `dev` is on.
pub fn visible(cat: &Catalog, dev: bool) -> Vec<App> {
    cat.apps
        .iter()
        .filter(|a| a.hub.is_some())
        .filter(|a| KNOWN_STATUS.contains(&a.status.as_str()))
        .filter(|a| dev || a.status == "released")
        .cloned()
        .collect()
}
