//! FastKiwi Hub, Phase C prototype shell: the testable core (catalog, detection, module
//! protocol, navigation allowlist, file serving). The window and WebViews are in `main.rs`.

pub mod allowlist;
pub mod catalog;
pub mod detect;
pub mod protocol;
pub mod serve;
pub mod settings;
#[cfg(windows)]
pub mod win;
