//! `check-app <app.exe> [--app <id>] [--update-silence <seconds>] [--no-window-check]`
//!
//! Runs the fastkiwi-hub-cli@1 conformance checks (docs/HUB-APP-CONTRACT.md §9) against an app
//! build and prints one line per check. Exit code 0: conforms; 1: a check failed; 2: usage.
//!
//! It runs `--hub-update --check-only`, which may ask the app's update source whether a newer
//! version exists (the app's own, consented check). It never downloads or installs.

use std::path::PathBuf;
use std::time::Duration;

fn usage() -> ! {
    eprintln!(
        "usage: check-app <app.exe> [--app <id>] [--update-silence <seconds>] [--no-window-check]"
    );
    std::process::exit(2);
}

fn main() {
    let mut args = std::env::args().skip(1);
    let mut exe: Option<PathBuf> = None;
    let mut opts = conformance::Options::default();
    while let Some(a) = args.next() {
        match a.as_str() {
            "--app" => opts.app = Some(args.next().unwrap_or_else(|| usage())),
            "--update-silence" => {
                let s: u64 = args
                    .next()
                    .and_then(|s| s.parse().ok())
                    .unwrap_or_else(|| usage());
                opts.update_silence = Duration::from_secs(s);
            }
            "--no-window-check" => opts.check_windows = false,
            "-h" | "--help" => usage(),
            _ if a.starts_with('-') => usage(),
            _ if exe.is_none() => exe = Some(PathBuf::from(a)),
            _ => usage(),
        }
    }
    let Some(exe) = exe else { usage() };
    if !exe.is_file() {
        eprintln!("{}: not a file", exe.display());
        std::process::exit(2);
    }
    let report = conformance::check_app(&exe, &opts);
    print!("{}", report.to_text());
    std::process::exit(if report.passed() { 0 } else { 1 });
}
