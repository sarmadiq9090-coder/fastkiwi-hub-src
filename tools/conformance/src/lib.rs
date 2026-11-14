//! Conformance harness for `fastkiwi-hub-cli@1` (docs/HUB-APP-CONTRACT.md).
//!
//! [`check_app`] runs an app's exe through every hub command it declares in its
//! `fastkiwi-hub.json` and returns a [`Report`] of numbered checks. `check-app` is the
//! command-line front end; `stub-app` is a conforming app that the tests also break on purpose.
//!
//! This is a developer tool: it starts processes and talks HTTP to 127.0.0.1. None of this code
//! is part of the Hub exe.

pub mod checks;
pub mod http;
pub mod proc;

pub use checks::{Item, Options, Outcome, Report, check_app};

use sha2::{Digest, Sha256};

/// Lowercase hex SHA-256 of `bytes`.
pub fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
