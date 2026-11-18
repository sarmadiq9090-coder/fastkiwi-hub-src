//! The catalog signature: a minisign (Ed25519) signature over the exact bytes of
//! `apps.json`, by one of two public keys pinned here (the second is for key rotation). The
//! catalog can never add or replace a key. A fetched or cached catalog without a good
//! signature is ignored, and the Hub keeps the copy inside the exe ("fail closed").
//!
//! The signature's trusted comment must say `fastkiwi-hub-catalog serial=<n>` with the same
//! serial as the file, so a signature cannot be moved to another catalog version.

use minisign_verify::{PublicKey, Signature};

/// `keys/catalog-1.pub` (primary) and `keys/catalog-2.pub` (backup). Changing these needs a
/// Hub release.
pub const PINNED_KEYS: [&str; 2] = [
    "RWSoCoUN9B2680aBwm2L0XlTTNV3wkvn5w3RO+r5+tDFcw6NgePWZDWY",
    "RWRBG0e5TcwZ10JwP1musk95qtRDzwOqYdu9Gy8D4F7rQeD/kvQF53lp",
];

/// Largest signature file accepted.
pub const MAX_SIG: usize = 4096;

/// Check `sig` (the `.minisig` text) over `data` with a pinned key, and that its trusted
/// comment names `serial`.
pub fn verify_with(keys: &[&str], data: &[u8], sig: &str, serial: u64) -> Result<(), String> {
    if sig.len() > MAX_SIG {
        return Err("signature file too large".into());
    }
    let s = Signature::decode(sig).map_err(|e| format!("signature: {e}"))?;
    if s.trusted_comment() != format!("fastkiwi-hub-catalog serial={serial}") {
        return Err("signature is for another catalog serial".into());
    }
    let mut last = "no pinned key".to_string();
    for k in keys {
        let pk = PublicKey::from_base64(k).map_err(|e| format!("pinned key: {e}"))?;
        match pk.verify(data, &s, false) {
            Ok(()) => return Ok(()),
            Err(e) => last = e.to_string(),
        }
    }
    Err(format!("bad signature: {last}"))
}

pub fn verify(data: &[u8], sig: &str, serial: u64) -> Result<(), String> {
    verify_with(&PINNED_KEYS, data, sig, serial)
}
