//! A [`SystemView`] built from a JSON fixture, so detection can be tested without touching the
//! real registry or disk. Format (every map is matched without regard to case):
//!
//! ```json
//! {
//!   "env":      { "LOCALAPPDATA": "C:\\Users\\T\\AppData\\Local" },
//!   "files":    { "C:\\...\\bahth.exe": { "size": 6873600, "version": "1.1.0" } },
//!   "dirs":     { "C:\\...\\WinGet\\Packages": ["Sarmad.InstaFind_Microsoft.Winget.Source_8wekyb3d8bbwe"] },
//!   "links":    { "C:\\...\\WinGet\\Links\\bahth.exe": "C:\\...\\bahth.exe" },
//!   "registry": { "HKCU\\Software\\...\\Uninstall\\key": { "DisplayVersion": "1.6.1", "EstimatedSize": 328105 } },
//!   "packages": { "FastKiwi.InstaFind_sm1xjmj6v76yc": { "version": "1.1.0.0", "location": "C:\\Program Files\\WindowsApps\\x" } },
//!   "dir_sizes": { "C:\\Program Files\\WindowsApps\\x": 10485760 }
//! }
//! ```
//! Registry values: a JSON string is `REG_SZ`, a number is `REG_DWORD`.

use super::{FileFacts, Hive, PackageFacts, RegValue, SystemView, norm};
use serde::Deserialize;
use std::collections::BTreeMap;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct Raw {
    #[serde(default)]
    env: BTreeMap<String, String>,
    #[serde(default)]
    files: BTreeMap<String, RawFile>,
    #[serde(default)]
    dirs: BTreeMap<String, Vec<String>>,
    #[serde(default)]
    links: BTreeMap<String, String>,
    #[serde(default)]
    registry: BTreeMap<String, BTreeMap<String, serde_json::Value>>,
    #[serde(default)]
    packages: BTreeMap<String, RawPackage>,
    #[serde(default)]
    dir_sizes: BTreeMap<String, u64>,
    #[serde(default, rename = "_comment")]
    _comment: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawFile {
    size: u64,
    #[serde(default)]
    version: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawPackage {
    version: String,
    location: String,
}

#[derive(Debug, Default)]
pub struct FixtureView {
    env: BTreeMap<String, String>,
    files: BTreeMap<String, FileFacts>,
    dirs: BTreeMap<String, Vec<String>>,
    links: BTreeMap<String, String>,
    registry: BTreeMap<(Hive, String), BTreeMap<String, RegValue>>,
    packages: BTreeMap<String, PackageFacts>,
    dir_sizes: BTreeMap<String, u64>,
}

impl FixtureView {
    pub fn from_json(json: &str) -> Result<FixtureView, String> {
        let raw: Raw = serde_json::from_str(json).map_err(|e| e.to_string())?;
        let mut registry = BTreeMap::new();
        for (k, vals) in raw.registry {
            let (hive, sub) = k.split_once('\\').ok_or(format!("bad key {k}"))?;
            let hive = Hive::parse(hive).ok_or(format!("bad hive in {k}"))?;
            let mut m = BTreeMap::new();
            for (name, v) in vals {
                let rv = match v {
                    serde_json::Value::String(s) => RegValue::Str(s),
                    serde_json::Value::Number(n) => RegValue::Dword(
                        n.as_u64()
                            .and_then(|x| u32::try_from(x).ok())
                            .ok_or(format!("bad DWORD {name}"))?,
                    ),
                    _ => return Err(format!("bad value {name} in {k}")),
                };
                m.insert(name, rv);
            }
            registry.insert((hive, norm(sub)), m);
        }
        Ok(FixtureView {
            env: raw
                .env
                .into_iter()
                .map(|(k, v)| (k.to_uppercase(), v))
                .collect(),
            files: raw
                .files
                .into_iter()
                .map(|(k, f)| {
                    (
                        norm(&k),
                        FileFacts {
                            size: f.size,
                            version: f.version,
                        },
                    )
                })
                .collect(),
            dirs: raw.dirs.into_iter().map(|(k, v)| (norm(&k), v)).collect(),
            links: raw.links.into_iter().map(|(k, v)| (norm(&k), v)).collect(),
            registry,
            packages: raw
                .packages
                .into_iter()
                .map(|(k, p)| {
                    (
                        k.to_lowercase(),
                        PackageFacts {
                            version: p.version,
                            location: p.location,
                        },
                    )
                })
                .collect(),
            dir_sizes: raw
                .dir_sizes
                .into_iter()
                .map(|(k, v)| (norm(&k), v))
                .collect(),
        })
    }
}

impl SystemView for FixtureView {
    fn env(&self, name: &str) -> Option<String> {
        self.env.get(&name.to_uppercase()).cloned()
    }
    fn file(&self, path: &str) -> Option<FileFacts> {
        self.files.get(&norm(path)).cloned()
    }
    fn list_dir(&self, path: &str) -> Vec<String> {
        self.dirs.get(&norm(path)).cloned().unwrap_or_default()
    }
    fn read_link(&self, path: &str) -> Option<String> {
        self.links.get(&norm(path)).cloned()
    }
    fn reg_values(&self, hive: Hive, key: &str) -> Option<BTreeMap<String, RegValue>> {
        self.registry.get(&(hive, norm(key))).cloned()
    }
    fn store_package(&self, pfn: &str) -> Option<PackageFacts> {
        self.packages.get(&pfn.to_lowercase()).cloned()
    }
    fn dir_size(&self, path: &str) -> Option<u64> {
        self.dir_sizes.get(&norm(path)).copied()
    }
}
