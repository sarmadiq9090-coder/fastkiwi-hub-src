//! The harness against the stub app: the conforming stub passes every check, and each fault
//! (one broken rule) makes exactly the check for that rule fail.

use conformance::{Options, Outcome, Report, check_app};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

const ALL: &str = r#"["hub-host","hub-update","export-data"]"#;

struct Stage(PathBuf);

impl Stage {
    /// A temp folder with a copy of stub-app, its fastkiwi-hub.json and its fault file.
    fn new(app: &str, commands: &str, faults: &str) -> Stage {
        static N: AtomicU32 = AtomicU32::new(0);
        let dir = std::env::temp_dir().join(format!(
            "fk-stub-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::SeqCst)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::copy(env!("CARGO_BIN_EXE_stub-app"), dir.join("stub-app.exe")).unwrap();
        std::fs::write(
            dir.join("fastkiwi-hub.json"),
            format!(r#"{{"proto":"fastkiwi-hub-cli@1","app":"{app}","commands":{commands}}}"#),
        )
        .unwrap();
        if !faults.is_empty() {
            std::fs::write(dir.join("stub-fault.txt"), faults).unwrap();
        }
        Stage(dir)
    }
    fn exe(&self) -> PathBuf {
        self.0.join("stub-app.exe")
    }
}

impl Drop for Stage {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn run(faults: &str) -> Report {
    let s = Stage::new("stub", ALL, faults);
    check_app(&s.exe(), &opts())
}

fn opts() -> Options {
    Options {
        update_silence: Duration::from_secs(2),
        ..Options::default()
    }
}

fn failing(r: &Report) -> Vec<&'static str> {
    r.items
        .iter()
        .filter(|i| matches!(i.outcome, Outcome::Fail(_)))
        .map(|i| i.id)
        .collect()
}

/// The fault breaks `id`, and only `id`.
fn breaks(fault: &str, id: &str) {
    let r = run(fault);
    assert_eq!(failing(&r), vec![id], "fault {fault}:\n{}", r.to_text());
}

#[test]
fn conforming_stub_passes_everything() {
    let r = run("");
    assert!(r.passed(), "{}", r.to_text());
    assert_eq!(r.items.len(), 19);
    for i in &r.items {
        if i.id == "H12" && !cfg!(windows) {
            continue;
        }
        assert!(matches!(i.outcome, Outcome::Pass(_)), "{}", r.to_text());
    }
    assert!(r.to_text().ends_with("RESULT conforms\n"));
}

#[test]
fn id_follows_the_manifest() {
    let s = Stage::new("lemon", ALL, "");
    let r = check_app(&s.exe(), &opts());
    assert!(r.passed(), "{}", r.to_text());
    assert_eq!(r.app.as_deref(), Some("lemon"));
}

#[test]
fn token_guard() {
    breaks("no-key-check", "H3");
}

#[test]
fn weak_key() {
    let r = run("weak-key");
    assert!(r.failed("H1"));
    let Outcome::Fail(why) = &r.get("H1").unwrap().outcome else {
        unreachable!()
    };
    assert!(why.contains("key"), "{why}");
}

#[test]
fn handshake_with_an_extra_field() {
    breaks("bad-handshake", "H1");
}

#[test]
fn host_guard() {
    breaks("any-host", "H8");
}

#[test]
fn origin_guard() {
    breaks("any-origin", "H9");
}

#[test]
fn cors_wildcard() {
    breaks("cors-wildcard", "H10");
}

#[test]
fn page_without_csp() {
    breaks("no-csp", "H6");
}

#[test]
fn patched_embed_js() {
    breaks("embed-modified", "H7");
}

#[test]
fn extra_stdout() {
    breaks("extra-stdout", "H13");
}

#[test]
fn ignores_stdin() {
    breaks("ignore-stdin", "H14");
}

#[test]
fn listens_beyond_loopback() {
    let r = run("bind-all");
    let (Outcome::Pass(note) | Outcome::Fail(note) | Outcome::Skip(note)) =
        &r.get("H2").unwrap().outcome;
    if r.failed("H2") {
        assert_eq!(failing(&r), vec!["H2"], "{}", r.to_text());
    } else {
        // No routed non-loopback address on this machine: only [::1] could be tried.
        assert!(!note.contains(','), "{}", r.to_text());
    }
}

#[test]
fn bad_progress_value() {
    breaks("bad-progress", "U1");
}

#[test]
fn exit_code_must_match_the_last_line() {
    breaks("update-exit-mismatch", "U1");
}

#[test]
fn silent_updater() {
    breaks("update-silent", "U1");
}

#[test]
fn offline_check_is_still_conformant() {
    let r = run("update-offline");
    assert!(r.passed(), "{}", r.to_text());
    assert_eq!(
        r.get("U1").unwrap().outcome,
        Outcome::Pass("error offline".into())
    );
    let r = run("update-none");
    assert_eq!(r.get("U1").unwrap().outcome, Outcome::Pass("none".into()));
}

#[test]
fn export_with_an_unlisted_file() {
    breaks("export-extra-file", "E1");
}

#[test]
fn export_with_a_wrong_hash() {
    breaks("export-bad-hash", "E1");
}

#[test]
fn missing_or_foreign_manifest() {
    let s = Stage::new("stub", ALL, "");
    std::fs::remove_file(s.0.join("fastkiwi-hub.json")).unwrap();
    let r = check_app(&s.exe(), &opts());
    assert_eq!(failing(&r), vec!["M1"]);
    assert!(
        r.items
            .iter()
            .skip(1)
            .all(|i| matches!(i.outcome, Outcome::Skip(_)))
    );

    let s = Stage::new("stub", ALL, "");
    let o = Options {
        app: Some("lemon".into()),
        ..opts()
    };
    assert_eq!(failing(&check_app(&s.exe(), &o)), vec!["M1"]);
}

#[test]
fn only_declared_commands_run() {
    // Remember Me: no hosting, only the update helper and export.
    let s = Stage::new("remember-me", r#"["hub-update","export-data"]"#, "");
    let r = check_app(&s.exe(), &opts());
    assert!(r.passed(), "{}", r.to_text());
    for id in ["H1", "H7", "H14"] {
        assert!(matches!(r.get(id).unwrap().outcome, Outcome::Skip(_)));
    }
    assert!(matches!(r.get("U1").unwrap().outcome, Outcome::Pass(_)));
    assert!(matches!(r.get("E1").unwrap().outcome, Outcome::Pass(_)));
}

#[test]
fn check_app_cli() {
    let s = Stage::new("stub", ALL, "");
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_check-app"))
        .arg(s.exe())
        .arg("--update-silence")
        .arg("5")
        .output()
        .unwrap();
    let text = String::from_utf8(out.stdout).unwrap();
    assert_eq!(out.status.code(), Some(0), "{text}");
    assert!(text.contains("PASS H5"));

    let s = Stage::new("stub", ALL, "cors-wildcard");
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_check-app"))
        .arg(s.exe())
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8(out.stdout).unwrap().contains("FAIL H10"));

    let out = std::process::Command::new(env!("CARGO_BIN_EXE_check-app"))
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
}

#[test]
fn embed_js_is_the_pinned_release() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../sdk/embed.js");
    let bytes = std::fs::read(&path).unwrap();
    assert!(
        !bytes.contains(&b'\r'),
        "embed.js must keep LF line endings"
    );
    let last = hubcli::EMBED_RELEASES.last().unwrap();
    assert_eq!(
        conformance::sha256_hex(&bytes),
        last.sha256,
        "sdk/embed.js changed: add a new EMBED_RELEASES entry and bump its VERSION"
    );
    let text = String::from_utf8(bytes).unwrap();
    assert!(text.starts_with("// SPDX-License-Identifier: LicenseRef-Proprietary\n"));
    assert!(text.contains(&format!("export const VERSION = \"{}\";", last.version)));
    assert!(text.contains(&format!("fastkiwi-embed {}", last.version)));
    let sums = std::fs::read_to_string(path.with_file_name("embed.js.sha256")).unwrap();
    assert_eq!(sums, format!("{}  embed.js\n", last.sha256));
}
