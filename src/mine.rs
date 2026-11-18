//! «برامجي المضافة» / "Programs you added": any program already on this PC, picked by the
//! person (an `.exe` or a Start-menu shortcut). Stored only in the Hub's own folder
//! (`my-programs.json`), removable, and launched only from the exact path that was picked.
//! The Hub never copies, changes or installs these programs.

use serde::{Deserialize, Serialize};
use std::path::Path;

pub const FILE: &str = "my-programs.json";
/// Keeps the list (and the file) small.
pub const MAX: usize = 50;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Program {
    /// `my-<n>`. Catalog ids may not start with `my-`, so the two never meet.
    pub id: String,
    pub name: String,
    /// Absolute path of the `.exe` or `.lnk`.
    pub path: String,
    /// File name of its icon in the Hub's `icons` folder, if one was extracted.
    #[serde(default)]
    pub icon: Option<String>,
}

/// An absolute local path to an existing-looking `.exe` or `.lnk`: a drive letter, no UNC
/// share, no `..`, no stream, no quotes.
pub fn valid_path(p: &str) -> bool {
    let lower = p.to_ascii_lowercase();
    let b = p.as_bytes();
    p.len() >= 7
        && p.len() <= 260
        && b[0].is_ascii_alphabetic()
        && &p[1..3] == ":\\"
        && (lower.ends_with(".exe") || lower.ends_with(".lnk"))
        && !p[2..].contains([':', '"', '*', '?', '<', '>', '|', '/'])
        && !p.split('\\').any(|s| s == ".." || s == ".")
        && !p.chars().any(char::is_control)
}

fn valid_name(n: &str) -> bool {
    !n.trim().is_empty() && n.chars().count() <= 80 && !n.chars().any(char::is_control)
}

fn valid_icon(n: &str) -> bool {
    n.len() <= 40
        && n.ends_with(".png")
        && n.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'.')
        && !n.contains("..")
}

fn valid(p: &Program) -> bool {
    p.id.starts_with("my-")
        && crate::catalog::valid_id(&p.id)
        && valid_name(&p.name)
        && valid_path(&p.path)
        && p.icon.as_deref().is_none_or(valid_icon)
}

/// Parse the list; entries that do not pass the rules are dropped.
pub fn from_json(s: &str) -> Vec<Program> {
    let mut v: Vec<Program> = serde_json::from_str(s).unwrap_or_default();
    v.retain(valid);
    let mut seen = std::collections::BTreeSet::new();
    v.retain(|p| seen.insert(p.id.clone()));
    v.truncate(MAX);
    v
}

pub fn load(dir: &Path) -> Vec<Program> {
    std::fs::read_to_string(dir.join(FILE))
        .map(|s| from_json(&s))
        .unwrap_or_default()
}

pub fn save(dir: &Path, list: &[Program]) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let tmp = dir.join(format!("{FILE}.tmp"));
    std::fs::write(
        &tmp,
        serde_json::to_vec_pretty(list).map_err(std::io::Error::other)?,
    )?;
    std::fs::rename(tmp, dir.join(FILE))
}

/// The next free `my-<n>` id.
pub fn next_id(list: &[Program]) -> String {
    let n = list
        .iter()
        .filter_map(|p| p.id.strip_prefix("my-")?.parse::<u32>().ok())
        .max()
        .unwrap_or(0);
    format!("my-{}", n + 1)
}

/// A display name from the file name: `Notepad++.lnk` → `Notepad++`.
pub fn name_from_path(p: &str) -> String {
    let file = p.rsplit('\\').next().unwrap_or(p);
    let stem = file.rsplit_once('.').map_or(file, |(s, _)| s);
    stem.chars().take(80).collect()
}

/// Add a picked path, unless it is already in the list. Returns the new entry.
pub fn add(list: &mut Vec<Program>, path: &str, name: Option<String>) -> Result<Program, &'static str> {
    if !valid_path(path) {
        return Err("not an .exe or shortcut on a local drive");
    }
    if list.iter().any(|p| p.path.eq_ignore_ascii_case(path)) {
        return Err("already added");
    }
    if list.len() >= MAX {
        return Err("list is full");
    }
    let name = name
        .filter(|n| valid_name(n))
        .unwrap_or_else(|| name_from_path(path));
    let p = Program {
        id: next_id(list),
        name: if valid_name(&name) { name } else { "?".into() },
        path: path.to_string(),
        icon: None,
    };
    list.push(p.clone());
    Ok(p)
}

/// The icon file name for an entry.
pub fn icon_name(id: &str) -> String {
    format!("{id}.png")
}
