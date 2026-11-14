//! FastKiwi Hub (Phase D prototype): the testable core (catalog, detection, module
//! protocol, navigation allowlist, file serving, Phase D hosting rules and app processes).

pub mod allowlist;
pub mod catalog;
pub mod detect;
pub mod hosting;
pub mod hostproc;
pub mod protocol;
pub mod serve;
pub mod settings;
#[cfg(windows)]
pub mod win;
