//! FastKiwi Hub: the testable core (catalog, detection, links, update checks, programs the
//! person added, module protocol, navigation allowlist, file serving, hosting rules and app
//! processes).

pub mod allowlist;
pub mod catalog;
pub mod detect;
#[cfg(windows)]
pub mod findcli;
pub mod hosting;
pub mod hostproc;
pub mod hotkey;
pub mod links;
pub mod mine;
pub mod protocol;
pub mod serve;
pub mod settings;
pub mod trust;
pub mod updates;
#[cfg(windows)]
pub mod net;
#[cfg(windows)]
pub mod online;
#[cfg(windows)]
pub mod win;
