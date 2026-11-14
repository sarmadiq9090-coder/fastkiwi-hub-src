//! `fastkiwi-module@1`: the only channel between the Hub and a hosted module (docs/MODULE-CONTRACT.md).
//!
//! Transport: WebView2 web messages. Each message is one JSON object sent as a string
//! (`chrome.webview.postMessage(JSON.stringify(m))` in the module; `PostWebMessageAsString` in
//! the Hub). No host objects, no other bridge.
//!
//! Validation is strict and the same for both directions: the right `proto`, a known `type` for
//! that direction, exactly the fields that type has, values from closed sets, and at most
//! [`MAX_MESSAGE_BYTES`]. Anything else is dropped by the receiver.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

pub const PROTO: &str = "fastkiwi-module@1";
/// Largest message either side accepts (UTF-8 bytes of the JSON text).
pub const MAX_MESSAGE_BYTES: usize = 16 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Theme {
    Light,
    Dark,
    Midnight,
    Kiwi,
}

impl Theme {
    pub fn tone(self) -> Tone {
        match self {
            Theme::Light | Theme::Kiwi => Tone::Light,
            Theme::Dark | Theme::Midnight => Tone::Dark,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Tone {
    Light,
    Dark,
}

/// BCP 47 language tags the family ships.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Lang {
    Ar,
    En,
}

impl Lang {
    pub fn dir(self) -> Dir {
        match self {
            Lang::Ar => Dir::Rtl,
            Lang::En => Dir::Ltr,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Dir {
    Rtl,
    Ltr,
}

/// CLDR numbering-system ids.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Digits {
    /// Arabic-Indic digits ٠١٢٣٤٥٦٧٨٩.
    Arab,
    /// Latin digits 0123456789.
    Latn,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum UpdateState {
    Checking,
    Downloading,
    Installing,
    Done,
    RestartNeeded,
    Failed,
}

/// Hub → module.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case", deny_unknown_fields)]
pub enum HubMsg {
    Theme {
        theme: Theme,
        tone: Tone,
    },
    Lang {
        lang: Lang,
        dir: Dir,
    },
    Digits {
        digits: Digits,
    },
    UpdateProgress {
        state: UpdateState,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        progress: Option<f64>,
    },
}

/// Module → Hub.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case", deny_unknown_fields)]
pub enum ModuleMsg {
    /// First message after the page loads. The Hub answers with theme, lang and digits.
    Ready {
        module: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        version: Option<String>,
    },
    /// Ask the Hub to open the installed app in its own window (ShellExecute, fire and forget).
    OpenOwnWindow {},
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProtoError {
    TooLarge(usize),
    NotJson,
    NotObject,
    BadProto,
    MissingType,
    UnknownType(String),
    /// A known type sent the wrong way (e.g. a module sending `theme`).
    WrongDirection(String),
    BadFields(String),
    BadValue(String),
    WrongModule {
        expected: String,
        got: String,
    },
}

impl std::fmt::Display for ProtoError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TooLarge(n) => write!(f, "message is {n} bytes (max {MAX_MESSAGE_BYTES})"),
            Self::NotJson => write!(f, "message is not JSON"),
            Self::NotObject => write!(f, "message is not a JSON object"),
            Self::BadProto => write!(f, "proto is not {PROTO}"),
            Self::MissingType => write!(f, "type is missing"),
            Self::UnknownType(t) => write!(f, "unknown type {t:?}"),
            Self::WrongDirection(t) => write!(f, "type {t:?} is not allowed in this direction"),
            Self::BadFields(e) => write!(f, "bad fields: {e}"),
            Self::BadValue(e) => write!(f, "bad value: {e}"),
            Self::WrongModule { expected, got } => {
                write!(f, "module {got:?} is not the hosted module {expected:?}")
            }
        }
    }
}

impl std::error::Error for ProtoError {}

const HUB_TYPES: [&str; 4] = ["theme", "lang", "digits", "update-progress"];
const MODULE_TYPES: [&str; 2] = ["ready", "open-own-window"];

/// Common envelope checks; returns the object without `proto`.
fn envelope(
    text: &str,
    allowed: &[&str],
    other: &[&str],
) -> Result<Map<String, Value>, ProtoError> {
    if text.len() > MAX_MESSAGE_BYTES {
        return Err(ProtoError::TooLarge(text.len()));
    }
    let v: Value = serde_json::from_str(text).map_err(|_| ProtoError::NotJson)?;
    let Value::Object(mut obj) = v else {
        return Err(ProtoError::NotObject);
    };
    match obj.remove("proto") {
        Some(Value::String(p)) if p == PROTO => {}
        _ => return Err(ProtoError::BadProto),
    }
    let ty = match obj.get("type") {
        Some(Value::String(t)) => t.clone(),
        _ => return Err(ProtoError::MissingType),
    };
    if other.contains(&ty.as_str()) {
        return Err(ProtoError::WrongDirection(ty));
    }
    if !allowed.contains(&ty.as_str()) {
        return Err(ProtoError::UnknownType(ty));
    }
    // Exactly the fields of that type (serde's deny_unknown_fields is not relied on for
    // internally tagged enums).
    let fields = type_fields(&ty);
    if let Some(extra) = obj
        .keys()
        .find(|k| k.as_str() != "type" && !fields.contains(&k.as_str()))
    {
        return Err(ProtoError::BadFields(format!(
            "unknown field {extra:?} in {ty:?}"
        )));
    }
    Ok(obj)
}

fn type_fields(ty: &str) -> &'static [&'static str] {
    match ty {
        "theme" => &["theme", "tone"],
        "lang" => &["lang", "dir"],
        "digits" => &["digits"],
        "update-progress" => &["state", "progress"],
        "ready" => &["module", "version"],
        _ => &[],
    }
}

/// SemVer 2.0.0 core with optional pre-release and build metadata.
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

/// Parse a message a module sent to the Hub. `expected_module` is the id of the module the
/// sending WebView hosts; a `ready` that names another module is refused.
pub fn parse_module_msg(text: &str, expected_module: &str) -> Result<ModuleMsg, ProtoError> {
    let obj = envelope(text, &MODULE_TYPES, &HUB_TYPES)?;
    let msg: ModuleMsg = serde_json::from_value(Value::Object(obj))
        .map_err(|e| ProtoError::BadFields(e.to_string()))?;
    if let ModuleMsg::Ready { module, version } = &msg {
        if module != expected_module {
            return Err(ProtoError::WrongModule {
                expected: expected_module.to_string(),
                got: module.clone(),
            });
        }
        if let Some(v) = version
            && !is_semver(v)
        {
            return Err(ProtoError::BadValue(format!("version {v:?} is not SemVer")));
        }
    }
    Ok(msg)
}

/// Parse a message the Hub sent to a module (used by tests and to check what the Hub sends).
pub fn parse_hub_msg(text: &str) -> Result<HubMsg, ProtoError> {
    let obj = envelope(text, &HUB_TYPES, &MODULE_TYPES)?;
    let msg: HubMsg = serde_json::from_value(Value::Object(obj))
        .map_err(|e| ProtoError::BadFields(e.to_string()))?;
    check_hub_msg(&msg)?;
    Ok(msg)
}

fn check_hub_msg(msg: &HubMsg) -> Result<(), ProtoError> {
    match *msg {
        HubMsg::Theme { theme, tone } if theme.tone() != tone => Err(ProtoError::BadValue(
            format!("tone {tone:?} does not match theme {theme:?}"),
        )),
        HubMsg::Lang { lang, dir } if lang.dir() != dir => Err(ProtoError::BadValue(format!(
            "dir {dir:?} does not match lang {lang:?}"
        ))),
        HubMsg::UpdateProgress { state, progress } => {
            let needs = matches!(state, UpdateState::Downloading | UpdateState::Installing);
            match progress {
                Some(p) if !(0.0..=1.0).contains(&p) || !p.is_finite() => Err(
                    ProtoError::BadValue(format!("progress {p} is outside 0..1")),
                ),
                None if needs => Err(ProtoError::BadValue(format!(
                    "{state:?} needs a progress value"
                ))),
                _ => Ok(()),
            }
        }
        _ => Ok(()),
    }
}

/// Serialise a Hub → module message with the envelope. Panics only on a Hub bug (a message
/// that would fail its own validation), which tests cover.
pub fn encode_hub_msg(msg: &HubMsg) -> String {
    debug_assert!(check_hub_msg(msg).is_ok(), "hub built an invalid message");
    let mut v = serde_json::to_value(msg).expect("serialisable");
    if let Value::Object(o) = &mut v {
        o.insert("proto".into(), Value::String(PROTO.into()));
    }
    v.to_string()
}

/// Serialise a module → Hub message with the envelope (used by tests and the contract examples).
pub fn encode_module_msg(msg: &ModuleMsg) -> String {
    let mut v = serde_json::to_value(msg).expect("serialisable");
    if let Value::Object(o) = &mut v {
        o.insert("proto".into(), Value::String(PROTO.into()));
    }
    v.to_string()
}

/// The three settings messages the Hub sends after `ready` and after every change.
pub fn settings_msgs(theme: Theme, lang: Lang, digits: Digits) -> [HubMsg; 3] {
    [
        HubMsg::Theme {
            theme,
            tone: theme.tone(),
        },
        HubMsg::Lang {
            lang,
            dir: lang.dir(),
        },
        HubMsg::Digits { digits },
    ]
}
