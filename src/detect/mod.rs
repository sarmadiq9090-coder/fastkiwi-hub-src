//! Read-only detection of installed FastKiwi apps (HUB-DESIGN §4, "Detection per channel").
//!
//! The logic here is pure: it asks a [`SystemView`] for facts (files, registry values, Store
//! packages) and turns the catalog's probes into a list of installs. The real view
//! ([`windows::WindowsView`]) only opens keys with `KEY_READ` and only reads files; tests use
//! [`fixture::FixtureView`] with JSON fixtures. Nothing here writes anywhere.

pub mod fixture;
#[cfg(windows)]
pub mod windows;

use crate::catalog::{App, Installer, Probe};
use serde::Serialize;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Hive {
    /// HKEY_CURRENT_USER.
    Hkcu,
    /// HKEY_LOCAL_MACHINE, 64-bit view.
    Hklm,
    /// HKEY_LOCAL_MACHINE, 32-bit view (WOW6432Node).
    Hklm32,
}

impl Hive {
    pub fn parse(s: &str) -> Option<Hive> {
        match s.to_ascii_uppercase().as_str() {
            "HKCU" => Some(Hive::Hkcu),
            "HKLM" => Some(Hive::Hklm),
            "HKLM32" => Some(Hive::Hklm32),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegValue {
    Str(String),
    Dword(u32),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileFacts {
    pub size: u64,
    /// The exe's file version (`VS_FIXEDFILEINFO`), if it has one.
    pub version: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageFacts {
    pub version: String,
    pub location: String,
}

/// Read-only facts about this PC. Paths are Windows paths; comparisons ignore case.
pub trait SystemView {
    fn env(&self, name: &str) -> Option<String>;
    fn file(&self, path: &str) -> Option<FileFacts>;
    fn list_dir(&self, path: &str) -> Vec<String>;
    /// The target of a symbolic link, if `path` is one.
    fn read_link(&self, path: &str) -> Option<String>;
    /// All string and DWORD values of a key, or `None` when the key does not exist.
    fn reg_values(&self, hive: Hive, key: &str) -> Option<BTreeMap<String, RegValue>>;
    /// The current user's package with this family name.
    fn store_package(&self, pfn: &str) -> Option<PackageFacts>;
    fn dir_size(&self, path: &str) -> Option<u64>;
    /// A small text file (at most 4 KiB), for `fastkiwi-hub.json`. Read only.
    fn read_text(&self, path: &str) -> Option<String>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Source {
    Store,
    Nsis,
    Inno,
    Winget,
    Portable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Install {
    pub source: Source,
    pub version: Option<String>,
    pub size_bytes: Option<u64>,
    /// Folder of the install (display only).
    pub location: String,
    /// The exe to launch (non-Store copies).
    pub exe: Option<String>,
    /// `<pfn>!<app id>` (Store copies).
    pub aumid: Option<String>,
    /// False when the record exists but the exe is missing.
    pub launchable: bool,
    /// The entry page of a web module inside this install (kind S apps), if present.
    pub web_entry: Option<String>,
    /// How this copy was found (probe kind), for the "source" line.
    pub found_by: &'static str,
    /// The hub commands this copy declares in its `fastkiwi-hub.json` (fastkiwi-hub-cli@1),
    /// or `None` when it has no valid file for this app (an older build: own window only).
    pub hub: Option<Vec<hubcli::Command>>,
}

impl Install {
    pub fn declares(&self, c: hubcli::Command) -> bool {
        self.hub.as_ref().is_some_and(|h| h.contains(&c))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AppDetection {
    pub id: String,
    pub installs: Vec<Install>,
    /// A trace such as `HKCU\Software\BahthSari` exists, but no copy was found.
    pub used_before: bool,
}

const UNINSTALL: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\";
const RUN: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";

/// Expand a leading `%VAR%` (the only form the catalog allows).
pub fn expand(view: &dyn SystemView, path: &str) -> Option<String> {
    let rest = path.strip_prefix('%')?;
    let (var, tail) = rest.split_once('%')?;
    let base = view.env(var)?;
    Some(format!("{}{}", base.trim_end_matches('\\'), tail))
}

/// Lower case, backslashes, no trailing backslash: for comparing paths.
pub fn norm(path: &str) -> String {
    path.replace('/', "\\")
        .trim_end_matches('\\')
        .to_lowercase()
}

fn parent(path: &str) -> String {
    match path.rsplit_once('\\') {
        Some((p, _)) => p.to_string(),
        None => String::new(),
    }
}

fn join(dir: &str, name: &str) -> String {
    format!("{}\\{}", dir.trim_end_matches('\\'), name)
}

fn is_absolute(p: &str) -> bool {
    let b = p.as_bytes();
    b.len() > 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && b[2] == b'\\'
}

/// The exe path at the start of a command line (`"C:\x\a.exe" --startup` or `C:\x\a.exe -s`).
pub fn command_exe(cmd: &str) -> Option<String> {
    let cmd = cmd.trim();
    let exe = if let Some(rest) = cmd.strip_prefix('"') {
        rest.split_once('"')?.0.to_string()
    } else {
        let lower = cmd.to_ascii_lowercase();
        let end = lower.find(".exe")? + 4;
        cmd[..end].to_string()
    };
    (is_absolute(&exe) && exe.to_ascii_lowercase().ends_with(".exe")).then_some(exe)
}

/// The places a desktop copy may be launched from.
pub const LAUNCH_ROOTS: [&str; 3] = [
    "%PROGRAMFILES%",
    "%PROGRAMFILES(X86)%",
    "%LOCALAPPDATA%\\Programs",
];

/// Is `exe` an absolute path inside one of [`LAUNCH_ROOTS`], with no `.` or `..` segment?
pub fn allowed_exe(view: &dyn SystemView, exe: &str) -> bool {
    if !is_absolute(exe) || exe.split('\\').any(|s| s == ".." || s == ".") || exe.contains('/') {
        return false;
    }
    let e = norm(exe);
    LAUNCH_ROOTS.iter().filter_map(|r| expand(view, r)).any(|root| {
        let r = norm(&root);
        !r.is_empty() && e.starts_with(&format!("{r}\\"))
    })
}

fn str_value(vals: &BTreeMap<String, RegValue>, name: &str) -> Option<String> {
    vals.iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(name))
        .and_then(|(_, v)| match v {
            RegValue::Str(s) if !s.trim().is_empty() => Some(s.trim().to_string()),
            _ => None,
        })
}

fn dword_value(vals: &BTreeMap<String, RegValue>, name: &str) -> Option<u32> {
    vals.iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(name))
        .and_then(|(_, v)| match v {
            RegValue::Dword(d) => Some(*d),
            RegValue::Str(s) => s.trim().parse().ok(),
        })
}

fn web_entry(view: &dyn SystemView, location: &str, app: &App) -> Option<String> {
    let m = app.hub.as_ref()?.module.as_ref()?;
    let entry = join(&join(location, &m.web_dir), &m.entry);
    view.file(&entry).map(|_| entry)
}

fn exe_install(
    view: &dyn SystemView,
    app: &App,
    source: Source,
    exe: &str,
    found_by: &'static str,
) -> Option<Install> {
    let f = view.file(exe)?;
    let location = parent(exe);
    Some(Install {
        source,
        version: f.version,
        size_bytes: Some(f.size),
        web_entry: web_entry(view, &location, app),
        location,
        exe: Some(exe.to_string()),
        aumid: None,
        launchable: true,
        found_by,
        hub: None,
    })
}

fn probe_uninstall(
    view: &dyn SystemView,
    app: &App,
    key: &str,
    installer: Installer,
    exe_name: &str,
) -> Vec<Install> {
    let source = match installer {
        Installer::Nsis => Source::Nsis,
        Installer::Inno => Source::Inno,
    };
    let mut out = Vec::new();
    for hive in [Hive::Hkcu, Hive::Hklm, Hive::Hklm32] {
        let Some(vals) = view.reg_values(hive, &format!("{UNINSTALL}{key}")) else {
            continue;
        };
        // Inno writes InstallLocation in the uninstall key; electron-builder's NSIS writes it
        // under Software\<guid>; otherwise use the uninstaller's folder.
        let location = str_value(&vals, "InstallLocation")
            .or_else(|| {
                view.reg_values(hive, &format!("Software\\{key}"))
                    .and_then(|v| str_value(&v, "InstallLocation"))
            })
            .or_else(|| {
                str_value(&vals, "UninstallString")
                    .and_then(|c| command_exe(&c).map(|e| parent(&e)))
            })
            .or_else(|| {
                str_value(&vals, "DisplayIcon").map(|i| parent(i.split(',').next().unwrap_or("")))
            })
            .map(|l| l.trim_end_matches('\\').to_string());
        let Some(location) = location.filter(|l| is_absolute(l)) else {
            continue;
        };
        let exe = join(&location, exe_name);
        let exists = view.file(&exe).is_some();
        out.push(Install {
            source,
            version: str_value(&vals, "DisplayVersion"),
            size_bytes: dword_value(&vals, "EstimatedSize").map(|kb| u64::from(kb) * 1024),
            web_entry: web_entry(view, &location, app),
            location,
            exe: exists.then_some(exe),
            aumid: None,
            launchable: exists,
            found_by: "uninstall",
            hub: None,
        });
    }
    out
}

fn probe_winget(
    view: &dyn SystemView,
    app: &App,
    package_id: &str,
    link: &str,
    exe_name: &str,
) -> Vec<Install> {
    let mut out = Vec::new();
    let roots = [
        expand(view, "%LOCALAPPDATA%\\Microsoft\\WinGet"),
        expand(view, "%PROGRAMFILES%\\WinGet"),
    ];
    for root in roots.into_iter().flatten() {
        let l = join(&join(&root, "Links"), link);
        if view.file(&l).is_some() {
            let target = view.read_link(&l).unwrap_or(l);
            out.extend(exe_install(
                view,
                app,
                Source::Winget,
                &target,
                "winget-link",
            ));
        }
        let packages = join(&root, "Packages");
        let prefix = format!("{}_", package_id.to_lowercase());
        for d in view.list_dir(&packages) {
            if d.to_lowercase().starts_with(&prefix) {
                let exe = join(&join(&packages, &d), exe_name);
                out.extend(exe_install(
                    view,
                    app,
                    Source::Winget,
                    &exe,
                    "winget-package",
                ));
            }
        }
    }
    out
}

/// Run every probe of `app` and return what is on this PC. Both copies are listed when both
/// exist (HUB-DESIGN §4); the same exe found twice is listed once.
pub fn detect_app(view: &dyn SystemView, app: &App) -> AppDetection {
    let mut installs: Vec<Install> = Vec::new();
    let mut used_before = false;
    let Some(hub) = &app.hub else {
        return AppDetection {
            id: app.id.clone(),
            installs,
            used_before,
        };
    };
    for probe in &hub.detect {
        let found: Vec<Install> = match probe {
            Probe::Store { pfn, app_id } => view
                .store_package(pfn)
                .map(|p| Install {
                    source: Source::Store,
                    version: Some(p.version.clone()),
                    size_bytes: view.dir_size(&p.location),
                    web_entry: web_entry(view, &p.location, app),
                    location: p.location,
                    exe: None,
                    aumid: Some(format!("{pfn}!{app_id}")),
                    launchable: true,
                    found_by: "store",
                    hub: None,
                })
                .into_iter()
                .collect(),
            Probe::Uninstall {
                key,
                installer,
                exe,
            } => probe_uninstall(view, app, key, *installer, exe),
            Probe::Winget {
                package_id,
                link,
                exe,
            } => probe_winget(view, app, package_id, link, exe),
            Probe::Path { path } => expand(view, path)
                .and_then(|p| exe_install(view, app, Source::Portable, &p, "path"))
                .into_iter()
                .collect(),
            Probe::RunValue { name } => view
                .reg_values(Hive::Hkcu, RUN)
                .and_then(|v| str_value(&v, name))
                .and_then(|cmd| command_exe(&cmd))
                .and_then(|exe| exe_install(view, app, Source::Portable, &exe, "run-value"))
                .into_iter()
                .collect(),
            Probe::Unknown => Vec::new(),
            Probe::RegHint { key } => {
                if let Some((hive, sub)) = key.split_once('\\')
                    && let Some(h) = Hive::parse(hive)
                    && view.reg_values(h, sub).is_some()
                {
                    used_before = true;
                }
                Vec::new()
            }
        };
        for i in found {
            let dup = installs.iter().any(|j| {
                (i.exe.is_some() && i.exe.as_deref().map(norm) == j.exe.as_deref().map(norm))
                    || (i.aumid.is_some() && i.aumid == j.aumid)
                    || (i.exe.is_none()
                        && i.aumid.is_none()
                        && norm(&i.location) == norm(&j.location)
                        && i.source == j.source)
            });
            if !dup {
                installs.push(i);
            }
        }
    }
    // Only copies in fixed install places count (council 2026-11-18, C2): Program Files and
    // %LOCALAPPDATA%\Programs for desktop copies, the package family for Store copies. A copy
    // found through a registry value or a link that points elsewhere is not listed.
    installs.retain(|i| i.source == Source::Store || i.exe.as_deref().is_none_or(|e| allowed_exe(view, e)));
    if !installs.is_empty() {
        used_before = false;
    }
    for i in &mut installs {
        i.hub = hub_caps(view, app, i);
    }
    AppDetection {
        id: app.id.clone(),
        installs,
        used_before,
    }
}

/// The commands in the copy's `fastkiwi-hub.json` (next to its exe, or in its package folder).
/// A missing, invalid or foreign file means no hub mode.
fn hub_caps(view: &dyn SystemView, app: &App, i: &Install) -> Option<Vec<hubcli::Command>> {
    let dir = i
        .exe
        .as_deref()
        .map(parent)
        .unwrap_or_else(|| i.location.clone());
    let text = view.read_text(&join(&dir, hubcli::APP_MANIFEST))?;
    hubcli::parse_app_manifest(&text, Some(&app.id))
        .ok()
        .map(|m| m.commands)
}

/// Detect every Windows app in `apps` (phone apps are never looked for on the PC).
pub fn detect_all(view: &dyn SystemView, apps: &[App]) -> Vec<AppDetection> {
    apps.iter()
        .filter(|a| a.platform == crate::catalog::Platform::Windows)
        .map(|a| detect_app(view, a))
        .collect()
}
