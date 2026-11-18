//! catalog-sign: the maintainer's tool for the Hub catalog (docs/ADDING-AN-APP.md).
//!
//! ```text
//! catalog-sign keygen <secret-key-file> <public-key-file>
//! catalog-sign sign   <secret-key-file> <apps.json>       # writes apps.json.minisig
//! catalog-sign verify <public-key-file> <apps.json>
//! ```
//!
//! Keys are minisign keys. The secret key is written without a password, so keep it outside
//! every repository (the owner's `ops\secrets` folder) and in the password manager. The
//! signature's trusted comment carries the catalog's `serial`, which the Hub also checks.

use std::io::Cursor;
use std::process::ExitCode;

fn fail(msg: impl std::fmt::Display) -> ExitCode {
    eprintln!("catalog-sign: {msg}");
    ExitCode::from(2)
}

fn serial_of(json: &[u8]) -> Result<u64, String> {
    // A tiny read of the top-level "serial" (the tool has no JSON dependency on purpose).
    let text = std::str::from_utf8(json).map_err(|_| "apps.json is not UTF-8".to_string())?;
    let at = text.find("\"serial\"").ok_or("apps.json has no \"serial\"")?;
    let rest = text[at + 8..].trim_start().strip_prefix(':').ok_or("bad \"serial\"")?;
    let digits: String = rest.trim_start().chars().take_while(char::is_ascii_digit).collect();
    digits.parse().map_err(|_| "bad \"serial\"".to_string())
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
        ["keygen", sk_path, pk_path] => {
            if std::path::Path::new(sk_path).exists() {
                return fail(format!("{sk_path} exists; not overwriting a key"));
            }
            let kp = match minisign::KeyPair::generate_unencrypted_keypair() {
                Ok(k) => k,
                Err(e) => return fail(e),
            };
            let sk = match kp.sk.to_box(Some("FastKiwi Hub catalog secret key")) {
                Ok(b) => b.into_string(),
                Err(e) => return fail(e),
            };
            let pk = match kp.pk.to_box() {
                Ok(b) => b.into_string(),
                Err(e) => return fail(e),
            };
            if let Some(d) = std::path::Path::new(sk_path).parent() {
                let _ = std::fs::create_dir_all(d);
            }
            if let Err(e) = std::fs::write(sk_path, sk) {
                return fail(e);
            }
            if let Err(e) = std::fs::write(pk_path, &pk) {
                return fail(e);
            }
            println!("public key written to {pk_path}");
            ExitCode::SUCCESS
        }
        ["sign", sk_path, file] => {
            let sk_text = match std::fs::read_to_string(sk_path) {
                Ok(t) => t,
                Err(e) => return fail(e),
            };
            let sk = match minisign::SecretKeyBox::from_string(&sk_text)
                .and_then(|b| minisign::SecretKey::from_box(b, None))
            {
                Ok(k) => k,
                Err(e) => return fail(e),
            };
            let data = match std::fs::read(file) {
                Ok(d) => d,
                Err(e) => return fail(e),
            };
            let serial = match serial_of(&data) {
                Ok(s) => s,
                Err(e) => return fail(e),
            };
            let trusted = format!("fastkiwi-hub-catalog serial={serial}");
            let sig = match minisign::sign(
                None,
                &sk,
                Cursor::new(&data),
                Some(&trusted),
                Some("FastKiwi Hub catalog signature"),
            ) {
                Ok(s) => s,
                Err(e) => return fail(e),
            };
            let out = format!("{file}.minisig");
            if let Err(e) = std::fs::write(&out, sig.into_string()) {
                return fail(e);
            }
            println!("signed {file} (serial {serial}) -> {out}");
            ExitCode::SUCCESS
        }
        ["verify", pk_path, file] => {
            let pk = match std::fs::read_to_string(pk_path)
                .map_err(|e| e.to_string())
                .and_then(|t| minisign::PublicKeyBox::from_string(&t).map_err(|e| e.to_string()))
                .and_then(|b| b.into_public_key().map_err(|e| e.to_string()))
            {
                Ok(k) => k,
                Err(e) => return fail(e),
            };
            let data = match std::fs::read(file) {
                Ok(d) => d,
                Err(e) => return fail(e),
            };
            let sig = match std::fs::read_to_string(format!("{file}.minisig"))
                .map_err(|e| e.to_string())
                .and_then(|t| minisign::SignatureBox::from_string(&t).map_err(|e| e.to_string()))
            {
                Ok(s) => s,
                Err(e) => return fail(e),
            };
            match minisign::verify(&pk, &sig, Cursor::new(&data), true, false, false) {
                Ok(()) => {
                    println!("good signature");
                    ExitCode::SUCCESS
                }
                Err(e) => fail(e),
            }
        }
        _ => fail("usage: catalog-sign keygen <sk> <pk> | sign <sk> <apps.json> | verify <pk> <apps.json>"),
    }
}
