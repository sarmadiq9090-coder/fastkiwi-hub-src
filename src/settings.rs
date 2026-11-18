//! The Hub's own shared settings (HUB-DESIGN §3, «الإعدادات المشتركة»): language, theme, digits,
//! «follow», and which apps show on the home page. Stored as JSON in
//! `%LOCALAPPDATA%\FastKiwi\Hub\settings.json`, the Hub's own folder. Apps' folders are never
//! touched.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ThemeChoice {
    #[default]
    Auto,
    Light,
    Dark,
    Midnight,
    Kiwi,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum LangChoice {
    #[default]
    Auto,
    Ar,
    En,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum DigitsChoice {
    /// The FastKiwi default: Western digits (0123) in both languages.
    #[default]
    Auto,
    Arab,
    Latn,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub theme: ThemeChoice,
    pub lang: LangChoice,
    pub digits: DigitsChoice,
    /// «اجعل برامجي تتبع هذه الإعدادات». Only applies while hosted until spike S3 passes.
    pub follow: bool,
    /// App ids hidden from the home page.
    pub hidden: Vec<String>,
    /// «استعمل الإنترنت للتحقق من البرامج الجديدة والتحديثات»: on by default (D-H30), and
    /// disclosed in About and Settings. At most once a day while the Hub is open, the Hub asks
    /// fastkiwi.app for the catalog and GitHub for the latest releases. Off: nothing goes online
    /// unless the person presses «تحقق الآن».
    pub check_updates: bool,
    /// The bar's global shortcut (`hotkey::CHOICES`), or `off`.
    pub hotkey: String,
    /// Keep the last bar entries (off by default; clearable).
    pub bar_history: bool,
    /// Sites of third-party entries the person has agreed to open once (the first-time
    /// warning is not shown again for them).
    pub trusted_sites: Vec<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            theme: ThemeChoice::Auto,
            lang: LangChoice::Auto,
            digits: DigitsChoice::Auto,
            follow: true,
            hidden: Vec::new(),
            check_updates: true,
            hotkey: crate::hotkey::DEFAULT.to_string(),
            bar_history: false,
            trusted_sites: Vec::new(),
        }
    }
}

impl Settings {
    /// Parse a settings file; anything unreadable gives the defaults (never an error dialog).
    pub fn from_json(s: &str) -> Settings {
        let mut st: Settings = serde_json::from_str(s).unwrap_or_default();
        st.hidden.retain(|id| crate::catalog::valid_id(id));
        st.hidden.sort();
        st.hidden.dedup();
        if !crate::hotkey::CHOICES.contains(&st.hotkey.as_str()) {
            st.hotkey = crate::hotkey::DEFAULT.to_string();
        }
        st.trusted_sites.retain(|d| {
            !d.is_empty()
                && d.len() <= 253
                && d.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-')
        });
        st.trusted_sites.sort();
        st.trusted_sites.dedup();
        st.trusted_sites.truncate(200);
        st
    }
}

/// `%LOCALAPPDATA%\FastKiwi\Hub`, the Hub's own folder (settings and WebView2 profiles).
pub fn hub_dir() -> Option<std::path::PathBuf> {
    std::env::var_os("LOCALAPPDATA")
        .map(|d| std::path::PathBuf::from(d).join("FastKiwi").join("Hub"))
}

pub fn load() -> Settings {
    hub_dir()
        .and_then(|d| std::fs::read_to_string(d.join("settings.json")).ok())
        .map(|s| Settings::from_json(&s))
        .unwrap_or_default()
}

pub fn save(s: &Settings) -> std::io::Result<()> {
    let dir = hub_dir().ok_or_else(|| std::io::Error::other("no LOCALAPPDATA"))?;
    std::fs::create_dir_all(&dir)?;
    let tmp = dir.join("settings.json.tmp");
    std::fs::write(
        &tmp,
        serde_json::to_vec_pretty(s).map_err(std::io::Error::other)?,
    )?;
    std::fs::rename(tmp, dir.join("settings.json"))
}
