//! The Hub's host side (kiwi_hub::hostproc) against the real stub-app process: hosting starts,
//! fails and stops as the contract says, and update runs are read, checked and cancelled.

use hubcli::{UpdateCode, UpdateState};
use kiwi_hub::hostproc::{
    self, HostEvent, HostFailure, HostProcess, UpdateEnd, UpdateEvent, UpdateTiming,
};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::{Receiver, channel};
use std::time::Duration;

struct Stage(PathBuf);

impl Stage {
    fn new(app: &str, faults: &str) -> Stage {
        static N: AtomicU32 = AtomicU32::new(0);
        let dir = std::env::temp_dir().join(format!(
            "fk-hostside-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::SeqCst)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::copy(env!("CARGO_BIN_EXE_stub-app"), dir.join("stub-app.exe")).unwrap();
        std::fs::write(
            dir.join("fastkiwi-hub.json"),
            format!(
                r#"{{"proto":"fastkiwi-hub-cli@1","app":"{app}","commands":["hub-host","hub-update","export-data"]}}"#
            ),
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
        // The exe may still be closing; retry briefly.
        for _ in 0..20 {
            if std::fs::remove_dir_all(&self.0).is_ok() {
                return;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }
}

fn host(stage: &Stage, app: &str) -> (HostProcess, Receiver<HostEvent>) {
    let (tx, rx) = channel();
    let p = HostProcess::start(&stage.exe(), app, Duration::from_secs(10), move |e| {
        let _ = tx.send(e);
    })
    .unwrap();
    (p, rx)
}

const WAIT: Duration = Duration::from_secs(15);

#[test]
fn hosting_starts_and_stops() {
    let s = Stage::new("lemon", "");
    let (p, rx) = host(&s, "lemon");
    let HostEvent::Ready(h) = rx.recv_timeout(WAIT).unwrap() else {
        panic!("expected Ready");
    };
    assert_eq!(h.name, "lemon");
    assert!(h.enter_url().starts_with(&format!(
        "http://127.0.0.1:{}/__fastkiwi/enter?key=",
        h.port
    )));
    // The page the Hub's WebView opens answers with the session cookie.
    let r = conformance::http::request(
        h.port,
        "GET",
        &format!("{}?key={}", hubcli::ENTER_PATH, h.key),
        &[("Host", &format!("127.0.0.1:{}", h.port))],
    )
    .unwrap();
    assert_eq!(r.status, 303);
    // Closing the tab: stdin closes, and the app exits by itself with code 0.
    p.stop_with(Duration::from_secs(5));
    assert_eq!(rx.recv_timeout(WAIT).unwrap(), HostEvent::Exited(Some(0)));
}

#[test]
fn dropping_the_handle_stops_it_too() {
    let s = Stage::new("lemon", "");
    let (p, rx) = host(&s, "lemon");
    assert!(matches!(
        rx.recv_timeout(WAIT).unwrap(),
        HostEvent::Ready(_)
    ));
    drop(p);
    assert_eq!(rx.recv_timeout(WAIT).unwrap(), HostEvent::Exited(Some(0)));
}

#[test]
fn an_app_that_ignores_stdin_is_terminated() {
    let s = Stage::new("lemon", "ignore-stdin");
    let (p, rx) = host(&s, "lemon");
    assert!(matches!(
        rx.recv_timeout(WAIT).unwrap(),
        HostEvent::Ready(_)
    ));
    p.stop_with(Duration::from_millis(500));
    // Terminated: no clean exit code 0.
    assert_ne!(rx.recv_timeout(WAIT).unwrap(), HostEvent::Exited(Some(0)));
}

#[test]
fn bad_or_missing_handshakes_fall_back() {
    for (fault, want) in [
        ("bad-handshake", "bad"),
        ("weak-key", "bad"),
        ("no-handshake", "none"),
    ] {
        let s = Stage::new("lemon", fault);
        let (_p, rx) = host(&s, "lemon");
        match rx.recv_timeout(WAIT).unwrap() {
            HostEvent::Failed(HostFailure::BadHandshake(_)) if want == "bad" => {}
            HostEvent::Failed(HostFailure::NoHandshake) if want == "none" => {}
            e => panic!("{fault}: {e:?}"),
        }
        // Nothing follows a failure.
        assert!(rx.recv_timeout(Duration::from_millis(300)).is_err());
    }
    // A copy that reports the id of another app is refused too.
    let s = Stage::new("instafind", "");
    let (_p, rx) = host(&s, "lemon");
    assert!(matches!(
        rx.recv_timeout(WAIT).unwrap(),
        HostEvent::Failed(HostFailure::BadHandshake(_))
    ));
}

fn collect(rx: &Receiver<UpdateEvent>) -> Vec<UpdateEvent> {
    let mut evs = Vec::new();
    while let Ok(e) = rx.recv_timeout(WAIT) {
        let end = matches!(e, UpdateEvent::End(_));
        evs.push(e);
        if end {
            break;
        }
    }
    evs
}

fn update(stage: &Stage, check_only: bool, timing: UpdateTiming) -> Vec<UpdateEvent> {
    let (tx, rx) = channel();
    let _h = hostproc::start_update(&stage.exe(), check_only, 0, timing, move |e| {
        let _ = tx.send(e);
    })
    .unwrap();
    collect(&rx)
}

fn states(evs: &[UpdateEvent]) -> Vec<UpdateState> {
    evs.iter()
        .filter_map(|e| match e {
            UpdateEvent::Line(l) => Some(l.state),
            UpdateEvent::End(_) => None,
        })
        .collect()
}

fn end(evs: &[UpdateEvent]) -> &UpdateEnd {
    match evs.last() {
        Some(UpdateEvent::End(e)) => e,
        e => panic!("no end: {e:?}"),
    }
}

#[test]
fn full_update_streams_progress_then_done() {
    let s = Stage::new("lemon", "");
    let evs = update(&s, false, UpdateTiming::default());
    let st = states(&evs);
    assert_eq!(st.first(), Some(&UpdateState::Checking));
    assert!(st.contains(&UpdateState::Downloading));
    assert!(st.contains(&UpdateState::Installing));
    assert_eq!(st.last(), Some(&UpdateState::Done));
    let progress: Vec<f64> = evs
        .iter()
        .filter_map(|e| match e {
            UpdateEvent::Line(l) => l.progress,
            UpdateEvent::End(_) => None,
        })
        .collect();
    assert!(progress.windows(2).all(|w| w[0] <= w[1]));
    assert_eq!(progress.last(), Some(&1.0));
    let UpdateEnd::Final(l) = end(&evs) else {
        panic!("{evs:?}")
    };
    assert_eq!(l.state, UpdateState::Done);
    assert_eq!(l.version.as_deref(), Some("0.2.0"));
}

#[test]
fn check_only_says_available() {
    let s = Stage::new("lemon", "");
    let evs = update(&s, true, UpdateTiming::default());
    assert_eq!(
        states(&evs),
        vec![UpdateState::Checking, UpdateState::Available]
    );
    assert_eq!(end(&evs).shown().version.as_deref(), Some("0.2.0"));
}

#[test]
fn errors_and_broken_runs() {
    let s = Stage::new("lemon", "update-offline");
    let evs = update(&s, true, UpdateTiming::default());
    let shown = end(&evs).shown();
    assert_eq!(
        (shown.state, shown.code),
        (UpdateState::Error, Some(UpdateCode::Offline))
    );

    // A line outside the contract, an exit code that lies, or silence: error internal.
    for fault in ["bad-progress", "update-exit-mismatch", "update-silent"] {
        let s = Stage::new("lemon", fault);
        let timing = UpdateTiming {
            first_line: Duration::from_secs(5),
            silence: Duration::from_secs(1),
        };
        let evs = update(&s, true, timing);
        assert!(
            matches!(end(&evs), UpdateEnd::Broken(_)),
            "{fault}: {evs:?}"
        );
        let shown = end(&evs).shown();
        assert_eq!(
            (shown.state, shown.code),
            (UpdateState::Error, Some(UpdateCode::Internal)),
            "{fault}"
        );
    }
}

#[test]
fn cancel_before_installing() {
    let s = Stage::new("lemon", "");
    let (tx, rx) = channel();
    let h = hostproc::start_update(&s.exe(), false, 0, UpdateTiming::default(), move |e| {
        let _ = tx.send(e);
    })
    .unwrap();
    h.cancel();
    let evs = collect(&rx);
    let shown = end(&evs).shown();
    assert_eq!(
        (shown.state, shown.code),
        (UpdateState::Error, Some(UpdateCode::Cancelled)),
        "{evs:?}"
    );
    assert!(!states(&evs).contains(&UpdateState::Installing));
}

#[test]
fn the_hub_passes_exactly_the_contract_flags() {
    assert_eq!(
        hostproc::update_args(true, 0x1234),
        ["--hub-update", "--check-only", "--hub-owner", "4660"]
    );
    assert_eq!(
        hostproc::update_args(false, -5),
        ["--hub-update", "--hub-owner", "0"]
    );
}
