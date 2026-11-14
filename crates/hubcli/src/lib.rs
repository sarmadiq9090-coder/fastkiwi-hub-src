//! `fastkiwi-hub-cli@1`: the command-line contract between FastKiwi Hub and each FastKiwi app.
//! The text is `docs/HUB-APP-CONTRACT.md`; this crate is its machine-checked half.
//!
//! It only parses and validates. It does no I/O, starts no process and opens no socket, so the
//! Hub, the conformance harness (`tools/conformance`) and an app's own tests can all use it.
//!
//! Everything an app prints is one JSON object per line, UTF-8, at most [`MAX_LINE`] bytes,
//! with `"proto": "fastkiwi-hub-cli@1"`, exactly the fields its kind defines, values from closed
//! sets and no `null`. Anything else is refused.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::Duration;

/// Contract name and major version.
pub const PROTO: &str = "fastkiwi-hub-cli@1";
/// The capability file next to the app's exe. Without it the Hub never runs a hub command.
pub const APP_MANIFEST: &str = "fastkiwi-hub.json";
/// The manifest `--export-data` writes at the top of its output folder.
pub const EXPORT_MANIFEST: &str = "fastkiwi-export.json";
/// The folder inside the export folder that holds the app's files.
pub const EXPORT_DATA_DIR: &str = "data";
/// Largest line an app may print (UTF-8 bytes, without the line ending).
pub const MAX_LINE: usize = 1024;

/// Hub-host server: the one URL that takes the key and turns it into the session cookie.
pub const ENTER_PATH: &str = "/__fastkiwi/enter";
/// Hub-host server and kind S web folders: where the pinned `embed.js` is served.
pub const EMBED_PATH: &str = "/__fastkiwi/embed.js";
/// The session cookie that carries the key on every request.
pub const COOKIE: &str = "fkhub";
/// Length of the key: 32 random bytes as base64url without padding.
pub const KEY_LEN: usize = 43;

/// `--hub-host`: the handshake line must arrive within this time.
pub const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
/// `--hub-host`: the app must have exited this long after its stdin closed.
pub const EXIT_GRACE: Duration = Duration::from_secs(2);
/// `--hub-update`: the first line must arrive within this time.
pub const UPDATE_FIRST_LINE: Duration = Duration::from_secs(5);
/// `--hub-update`: the longest silence between two lines.
pub const UPDATE_SILENCE: Duration = Duration::from_secs(120);

/// Process exit codes of every hub command.
pub mod exit {
    /// Success. For `--hub-update`: the last line is `done`, `none` or `available`.
    pub const OK: i32 = 0;
    /// Failure. The last line is an `error` line with its `code`.
    pub const FAILED: i32 = 1;
    /// The command line was not understood. Nothing was printed and nothing was changed.
    pub const USAGE: i32 = 2;
}

/// A released `sdk/embed.js`: apps copy one of these byte for byte.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EmbedRelease {
    pub version: &'static str,
    /// Lowercase hex SHA-256 of the file as shipped (LF line endings).
    pub sha256: &'static str,
}

/// Every released `embed.js`, oldest first. The last entry is the current `sdk/embed.js`
/// (a test in `tools/conformance` fails if the file changes without a new entry).
pub const EMBED_RELEASES: &[EmbedRelease] = &[EmbedRelease {
    version: "1.0.0",
    sha256: "655ee287d4b152c08bc443281553232edf17765b0dc055187acdbdc81b05e28b",
}];

/// The hub commands an app can declare in its [`APP_MANIFEST`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Command {
    HubHost,
    HubUpdate,
    ExportData,
}

impl Command {
    /// The command-line flag.
    pub fn flag(self) -> &'static str {
        match self {
            Command::HubHost => "--hub-host",
            Command::HubUpdate => "--hub-update",
            Command::ExportData => "--export-data",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum CliError {
    TooLong(usize),
    NotJson,
    NotObject,
    /// A field holds `null` (absent fields are allowed where optional; `null` never is).
    Null(String),
    BadProto,
    BadFields(String),
    BadValue(String),
    WrongApp {
        expected: String,
        got: String,
    },
    /// `--hub-update` lines in an order the contract does not allow.
    Sequence(String),
    /// The exit code does not match the last line.
    ExitCode {
        expected: i32,
        got: Option<i32>,
    },
}

impl std::fmt::Display for CliError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TooLong(n) => write!(f, "line is {n} bytes (max {MAX_LINE})"),
            Self::NotJson => write!(f, "not JSON"),
            Self::NotObject => write!(f, "not a JSON object"),
            Self::Null(k) => write!(f, "field {k:?} is null"),
            Self::BadProto => write!(f, "proto is not {PROTO}"),
            Self::BadFields(e) => write!(f, "bad fields: {e}"),
            Self::BadValue(e) => write!(f, "bad value: {e}"),
            Self::WrongApp { expected, got } => write!(f, "app {got:?} is not {expected:?}"),
            Self::Sequence(e) => write!(f, "out of order: {e}"),
            Self::ExitCode { expected, got } => match got {
                Some(g) => write!(f, "exit code {g}, expected {expected}"),
                None => write!(f, "no exit code (killed), expected {expected}"),
            },
        }
    }
}

impl std::error::Error for CliError {}

/// Envelope checks shared by every line: size, one object, no `null`, the right `proto`. Then
/// a strict typed parse of the same text (unknown, missing and duplicate fields are refused).
fn strict<T: serde::de::DeserializeOwned>(text: &str) -> Result<T, CliError> {
    let text = text.strip_suffix('\n').unwrap_or(text);
    let text = text.strip_suffix('\r').unwrap_or(text);
    if text.len() > MAX_LINE {
        return Err(CliError::TooLong(text.len()));
    }
    let v: Value = serde_json::from_str(text).map_err(|_| CliError::NotJson)?;
    let Value::Object(obj) = &v else {
        return Err(CliError::NotObject);
    };
    if let Some((k, _)) = obj.iter().find(|(_, v)| v.is_null()) {
        return Err(CliError::Null(k.clone()));
    }
    if obj.get("proto").and_then(Value::as_str) != Some(PROTO) {
        return Err(CliError::BadProto);
    }
    serde_json::from_str(text).map_err(|e| CliError::BadFields(e.to_string()))
}

/// Strict version of [`strict`] for multi-line JSON documents (the export manifest, the app
/// manifest), which have no line-length limit but the same other rules.
fn strict_doc<T: serde::de::DeserializeOwned>(text: &str, max: usize) -> Result<T, CliError> {
    if text.len() > max {
        return Err(CliError::TooLong(text.len()));
    }
    let v: Value = serde_json::from_str(text).map_err(|_| CliError::NotJson)?;
    fn has_null(v: &Value) -> Option<String> {
        match v {
            Value::Null => Some(String::new()),
            Value::Array(a) => a.iter().find_map(has_null),
            Value::Object(o) => o
                .iter()
                .find_map(|(k, v)| has_null(v).map(|p| if p.is_empty() { k.clone() } else { p })),
            _ => None,
        }
    }
    let Value::Object(obj) = &v else {
        return Err(CliError::NotObject);
    };
    if let Some(k) = has_null(&v) {
        return Err(CliError::Null(k));
    }
    if obj.get("proto").and_then(Value::as_str) != Some(PROTO) {
        return Err(CliError::BadProto);
    }
    serde_json::from_str(text).map_err(|e| CliError::BadFields(e.to_string()))
}

/// A FastKiwi app id, as in the catalog: 1–32 of `a-z`, `0-9`, `-`.
pub fn valid_app_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 32
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

/// A hub-host key: exactly [`KEY_LEN`] base64url characters (32 random bytes, no padding).
pub fn valid_key(key: &str) -> bool {
    key.len() == KEY_LEN
        && key
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// SemVer 2.0.0: core with optional pre-release and build metadata.
pub fn is_semver(v: &str) -> bool {
    if v.len() > 64 {
        return false;
    }
    let (rest, build) = match v.split_once('+') {
        Some((r, b)) => (r, Some(b)),
        None => (v, None),
    };
    let (core, pre) = match rest.split_once('-') {
        Some((c, p)) => (c, Some(p)),
        None => (rest, None),
    };
    let nums: Vec<&str> = core.split('.').collect();
    let num_ok = |n: &str| {
        !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()) && (n == "0" || !n.starts_with('0'))
    };
    let ident_ok = |s: &str| {
        !s.is_empty()
            && s.split('.')
                .all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-'))
    };
    nums.len() == 3
        && nums.iter().all(|n| num_ok(n))
        && pre.is_none_or(|p| {
            ident_ok(p)
                && p.split('.').all(|id| {
                    !id.bytes().all(|b| b.is_ascii_digit()) || id == "0" || !id.starts_with('0')
                })
        })
        && build.is_none_or(ident_ok)
}

/// Lowercase hex SHA-256.
pub fn valid_sha256(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

// ---------------------------------------------------------------------------------------------
// fastkiwi-hub.json

/// `fastkiwi-hub.json`, next to the app's exe: which hub commands this build understands.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppManifest {
    pub proto: String,
    pub app: String,
    pub commands: Vec<Command>,
}

impl AppManifest {
    pub fn has(&self, c: Command) -> bool {
        self.commands.contains(&c)
    }
}

/// Parse `fastkiwi-hub.json`. `expected_app` is the catalog id the Hub detected at that path.
pub fn parse_app_manifest(text: &str, expected_app: Option<&str>) -> Result<AppManifest, CliError> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let m: AppManifest = strict_doc(text, 4096)?;
    if !valid_app_id(&m.app) {
        return Err(CliError::BadValue(format!("app id {:?}", m.app)));
    }
    if let Some(e) = expected_app
        && e != m.app
    {
        return Err(CliError::WrongApp {
            expected: e.into(),
            got: m.app,
        });
    }
    if m.commands.is_empty() {
        return Err(CliError::BadValue("commands is empty".into()));
    }
    for (i, c) in m.commands.iter().enumerate() {
        if m.commands[..i].contains(c) {
            return Err(CliError::BadValue(format!("{c:?} listed twice")));
        }
    }
    Ok(m)
}

// ---------------------------------------------------------------------------------------------
// --hub-host

/// The one line `--hub-host` prints on stdout.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Handshake {
    pub proto: String,
    /// The ephemeral port the app's UI server listens on, on 127.0.0.1 only.
    pub port: u16,
    /// Per-run secret; see [`valid_key`].
    pub key: String,
    /// The app's catalog id.
    pub name: String,
    /// The app's version (SemVer).
    pub version: String,
}

impl Handshake {
    /// The URL the Hub opens first. The app answers it with the session cookie and a redirect.
    pub fn enter_url(&self) -> String {
        format!(
            "http://127.0.0.1:{}{ENTER_PATH}?key={}",
            self.port, self.key
        )
    }
    /// `http://127.0.0.1:<port>`: the only origin the hosted WebView may show.
    pub fn origin(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }
}

pub fn parse_handshake(line: &str, expected_app: &str) -> Result<Handshake, CliError> {
    let h: Handshake = strict(line)?;
    if h.port < 1024 {
        return Err(CliError::BadValue(format!(
            "port {} is not ephemeral",
            h.port
        )));
    }
    if !valid_key(&h.key) {
        return Err(CliError::BadValue(format!(
            "key must be {KEY_LEN} base64url characters"
        )));
    }
    if h.name != expected_app {
        return Err(CliError::WrongApp {
            expected: expected_app.into(),
            got: h.name,
        });
    }
    if !is_semver(&h.version) {
        return Err(CliError::BadValue(format!(
            "version {:?} is not SemVer",
            h.version
        )));
    }
    Ok(h)
}

// ---------------------------------------------------------------------------------------------
// --hub-update

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum UpdateState {
    Checking,
    /// Final, `--check-only` only: a newer version exists (`version` says which).
    Available,
    Downloading,
    Installing,
    /// Final: the new version is installed.
    Done,
    /// Final: already up to date.
    None,
    /// Final: failed; `code` says why.
    Error,
}

impl UpdateState {
    pub fn is_final(self) -> bool {
        matches!(
            self,
            UpdateState::Available | UpdateState::Done | UpdateState::None | UpdateState::Error
        )
    }
}

/// Why an update failed. Fixed set: no free text ever crosses to the Hub.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum UpdateCode {
    /// No network, or the update source cannot be reached.
    Offline,
    /// The update source answered with something unusable (HTTP error, bad file).
    Server,
    /// The update metadata is invalid, expired, or older than what is installed (rollback).
    Metadata,
    /// The download failed its signature or hash check.
    Signature,
    DiskFull,
    /// Files are locked (for example the app is running and cannot be replaced).
    InUse,
    /// The user declined a Windows dialog (Store consent, UAC).
    Declined,
    /// Updates are blocked by policy (Store updates off by an administrator, metered network).
    Policy,
    /// This copy cannot update itself (winget, Scoop, a developer build).
    Unsupported,
    /// Another update of this app is already running.
    Busy,
    /// The Hub closed stdin before `installing`.
    Cancelled,
    Internal,
}

/// One line of `--hub-update` output.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpdateLine {
    pub proto: String,
    pub state: UpdateState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub progress: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<UpdateCode>,
}

impl UpdateLine {
    pub fn new(state: UpdateState) -> Self {
        UpdateLine {
            proto: PROTO.into(),
            state,
            progress: None,
            version: None,
            code: None,
        }
    }
    pub fn to_line(&self) -> String {
        serde_json::to_string(self).expect("serialisable")
    }
}

/// Which fields each state takes: (progress, version, code), each `Need`, `May` or `No`.
#[derive(Clone, Copy, PartialEq)]
enum F {
    Need,
    May,
    No,
}

fn update_fields(s: UpdateState) -> (F, F, F) {
    use F::*;
    match s {
        UpdateState::Checking => (No, No, No),
        UpdateState::Available => (No, Need, No),
        UpdateState::Downloading | UpdateState::Installing => (Need, May, No),
        UpdateState::Done => (No, May, No),
        UpdateState::None => (No, No, No),
        UpdateState::Error => (No, No, Need),
    }
}

pub fn parse_update_line(line: &str) -> Result<UpdateLine, CliError> {
    let l: UpdateLine = strict(line)?;
    let (p, v, c) = update_fields(l.state);
    let check = |rule: F, present: bool, name: &str| match (rule, present) {
        (F::Need, false) => Err(CliError::BadFields(format!("{:?} needs {name}", l.state))),
        (F::No, true) => Err(CliError::BadFields(format!(
            "{:?} takes no {name}",
            l.state
        ))),
        _ => Ok(()),
    };
    check(p, l.progress.is_some(), "progress")?;
    check(v, l.version.is_some(), "version")?;
    check(c, l.code.is_some(), "code")?;
    if let Some(x) = l.progress
        && !(x.is_finite() && (0.0..=1.0).contains(&x))
    {
        return Err(CliError::BadValue(format!("progress {x} is outside 0..1")));
    }
    if let Some(v) = &l.version
        && !is_semver(v)
    {
        return Err(CliError::BadValue(format!("version {v:?} is not SemVer")));
    }
    Ok(l)
}

/// Checks the order of a `--hub-update` run, line by line, then its exit code.
#[derive(Debug, Clone)]
pub struct UpdateRun {
    check_only: bool,
    count: usize,
    last: Option<UpdateState>,
    progress: f64,
    last_line: Option<UpdateLine>,
}

impl UpdateRun {
    pub fn new(check_only: bool) -> Self {
        UpdateRun {
            check_only,
            count: 0,
            last: None,
            progress: 0.0,
            last_line: None,
        }
    }

    /// Add the next line. An error means the app broke the contract; the Hub then treats the
    /// run as failed (`internal`).
    pub fn push(&mut self, l: &UpdateLine) -> Result<(), CliError> {
        let seq = |m: &str| Err(CliError::Sequence(m.into()));
        if self.last.is_some_and(UpdateState::is_final) {
            return seq("a line after the final state");
        }
        match (self.last, l.state) {
            (None, UpdateState::Checking) => {}
            (None, _) => return seq("the first state must be checking"),
            (Some(_), UpdateState::Checking) => return seq("checking comes once, first"),
            (_, UpdateState::Available) if !self.check_only => {
                return seq("available is only for --check-only");
            }
            (_, UpdateState::Downloading | UpdateState::Installing | UpdateState::Done)
                if self.check_only =>
            {
                return seq("--check-only never downloads or installs");
            }
            (Some(prev), UpdateState::Available | UpdateState::None)
                if prev != UpdateState::Checking =>
            {
                return seq("available and none come right after checking");
            }
            (Some(UpdateState::Installing), UpdateState::Downloading) => {
                return seq("downloading after installing");
            }
            _ => {}
        }
        if let Some(p) = l.progress {
            if p < self.progress {
                return seq("progress went back");
            }
            self.progress = p;
        }
        self.count += 1;
        self.last = Some(l.state);
        self.last_line = Some(l.clone());
        Ok(())
    }

    /// The final line, once the process has exited with `exit_code` (`None`: it was killed).
    pub fn finish(&self, exit_code: Option<i32>) -> Result<&UpdateLine, CliError> {
        let Some(last) = self.last_line.as_ref().filter(|l| l.state.is_final()) else {
            return Err(CliError::Sequence("no final state".into()));
        };
        let expected = if last.state == UpdateState::Error {
            exit::FAILED
        } else {
            exit::OK
        };
        if exit_code != Some(expected) {
            return Err(CliError::ExitCode {
                expected,
                got: exit_code,
            });
        }
        Ok(last)
    }

    pub fn lines(&self) -> usize {
        self.count
    }
}

// ---------------------------------------------------------------------------------------------
// --export-data

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ExportState {
    Done,
    Error,
}

/// Why an export failed. Fixed set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ExportCode {
    /// The output folder exists and is not empty. Nothing was written.
    NotEmpty,
    DiskFull,
    AccessDenied,
    /// The app's data could not be read consistently (locked).
    InUse,
    Internal,
}

/// The one line `--export-data` prints when it ends.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExportResult {
    pub proto: String,
    pub state: ExportState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub files: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<ExportCode>,
}

pub fn parse_export_result(line: &str) -> Result<ExportResult, CliError> {
    let r: ExportResult = strict(line)?;
    let ok = match r.state {
        ExportState::Done => r.files.is_some() && r.bytes.is_some() && r.code.is_none(),
        ExportState::Error => r.files.is_none() && r.bytes.is_none() && r.code.is_some(),
    };
    if !ok {
        return Err(CliError::BadFields(
            "done takes files and bytes; error takes code".into(),
        ));
    }
    Ok(r)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExportFile {
    /// Relative to the export folder, `/`-separated, under `data/`.
    pub path: String,
    pub size: u64,
    pub sha256: String,
}

/// `fastkiwi-export.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExportManifest {
    pub proto: String,
    pub app: String,
    pub version: String,
    /// UTC, `YYYY-MM-DDTHH:MM:SSZ`.
    pub created: String,
    pub files: Vec<ExportFile>,
}

/// `YYYY-MM-DDTHH:MM:SSZ` with plausible ranges (RFC 3339, UTC, whole seconds).
pub fn valid_utc_stamp(s: &str) -> bool {
    let b = s.as_bytes();
    if b.len() != 20 {
        return false;
    }
    let shape = b"dddd-dd-ddTdd:dd:ddZ";
    if !b.iter().zip(shape).all(|(c, s)| {
        if *s == b'd' {
            c.is_ascii_digit()
        } else {
            c == s
        }
    }) {
        return false;
    }
    let n = |r: std::ops::Range<usize>| s[r].parse::<u32>().unwrap_or(99);
    (1..=12).contains(&n(5..7))
        && (1..=31).contains(&n(8..10))
        && n(11..13) < 24
        && n(14..16) < 60
        && n(17..19) < 61
}

/// A path inside an export: `data/<a>/<b>…`, `/`-separated, no empty, `.` or `..` part, no
/// backslash, colon, control or Windows-reserved character, no reserved device name.
pub fn safe_export_path(p: &str) -> bool {
    let Some(rest) = p.strip_prefix("data/") else {
        return false;
    };
    if p.len() > 240 || rest.is_empty() {
        return false;
    }
    rest.split('/').all(|part| {
        let stem = part.split('.').next().unwrap_or("").to_ascii_uppercase();
        let reserved = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
            || ((stem.starts_with("COM") || stem.starts_with("LPT"))
                && stem.len() == 4
                && stem.as_bytes()[3].is_ascii_digit());
        !part.is_empty()
            && part != "."
            && part != ".."
            && !part.ends_with('.')
            && !part.ends_with(' ')
            && !reserved
            && !part
                .chars()
                .any(|c| c.is_control() || "\\:*?\"<>|".contains(c))
    })
}

pub fn parse_export_manifest(text: &str, expected_app: &str) -> Result<ExportManifest, CliError> {
    let m: ExportManifest = strict_doc(text, 64 * 1024 * 1024)?;
    if m.app != expected_app {
        return Err(CliError::WrongApp {
            expected: expected_app.into(),
            got: m.app,
        });
    }
    if !is_semver(&m.version) {
        return Err(CliError::BadValue(format!("version {:?}", m.version)));
    }
    if !valid_utc_stamp(&m.created) {
        return Err(CliError::BadValue(format!("created {:?}", m.created)));
    }
    let mut seen = std::collections::HashSet::new();
    for f in &m.files {
        if !safe_export_path(&f.path) {
            return Err(CliError::BadValue(format!("path {:?}", f.path)));
        }
        if !valid_sha256(&f.sha256) {
            return Err(CliError::BadValue(format!("sha256 of {:?}", f.path)));
        }
        // Windows paths are case-insensitive: two entries that differ only in case collide.
        if !seen.insert(f.path.to_lowercase()) {
            return Err(CliError::BadValue(format!("{:?} listed twice", f.path)));
        }
    }
    Ok(m)
}
