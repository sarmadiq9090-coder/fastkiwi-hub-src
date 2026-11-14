//! Detection against JSON fixtures (no real registry or disk).

use kiwi_hub::catalog;
use kiwi_hub::detect::{self, AppDetection, Source, fixture::FixtureView};

fn run(fixture: &str, dev: bool) -> Vec<AppDetection> {
    let path = format!("{}/tests/fixtures/{fixture}", env!("CARGO_MANIFEST_DIR"));
    let view = FixtureView::from_json(&std::fs::read_to_string(path).unwrap()).unwrap();
    let cat = catalog::parse(catalog::FIXTURE).unwrap();
    detect::detect_all(&view, &catalog::visible(&cat, dev))
}

fn get<'a>(d: &'a [AppDetection], id: &str) -> &'a AppDetection {
    d.iter().find(|x| x.id == id).unwrap()
}

#[test]
fn this_pc() {
    let d = run("detect-this-pc.json", false);
    assert_eq!(
        d.len(),
        4,
        "InstaConvert is not detected in the shipped view"
    );

    let f = get(&d, "instafind");
    assert_eq!(f.installs.len(), 1);
    assert_eq!(f.installs[0].source, Source::Portable);
    assert_eq!(
        f.installs[0].exe.as_deref(),
        Some("C:\\Users\\T\\AppData\\Local\\Programs\\Bahth\\bahth.exe")
    );
    assert_eq!(f.installs[0].size_bytes, Some(6_873_600));
    assert_eq!(f.installs[0].version, None);

    assert!(get(&d, "instawords").installs.is_empty());

    let r = get(&d, "remember-me");
    assert_eq!(r.installs.len(), 1);
    let i = &r.installs[0];
    assert_eq!(i.source, Source::Nsis);
    assert_eq!(i.version.as_deref(), Some("1.6.1"));
    assert_eq!(i.size_bytes, Some(328_105 * 1024));
    assert_eq!(
        i.location,
        "C:\\Users\\T\\AppData\\Local\\Programs\\Remember Me"
    );
    assert!(i.launchable);

    let l = get(&d, "lemon");
    assert_eq!(l.installs[0].version.as_deref(), Some("2.2.0"));
    assert!(!l.used_before);
}

#[test]
fn every_channel_and_both_copies() {
    let d = run("detect-all-channels.json", false);

    let f = get(&d, "instafind");
    let srcs: Vec<Source> = f.installs.iter().map(|i| i.source).collect();
    assert_eq!(
        srcs,
        [Source::Store, Source::Winget],
        "link, package folder and Run value are one copy"
    );
    assert_eq!(
        f.installs[0].aumid.as_deref(),
        Some("FastKiwi.InstaFind_sm1xjmj6v76yc!InstaFind")
    );
    assert_eq!(f.installs[0].version.as_deref(), Some("1.1.0.0"));
    assert_eq!(f.installs[0].size_bytes, Some(10_485_760));
    assert_eq!(f.installs[1].version.as_deref(), Some("1.0.9"));
    assert_eq!(f.installs[1].found_by, "winget-link");
    assert!(!f.used_before, "a found copy wins over the trace");

    let w = get(&d, "instawords");
    assert_eq!(w.installs.len(), 1);
    let i = &w.installs[0];
    assert_eq!(i.source, Source::Inno);
    assert_eq!(
        i.location,
        "C:\\Users\\S\\AppData\\Local\\Programs\\InstaWords"
    );
    assert_eq!(i.size_bytes, Some(3100 * 1024));
    assert_eq!(
        i.web_entry.as_deref(),
        Some("C:\\Users\\S\\AppData\\Local\\Programs\\InstaWords\\web\\index.html"),
        "kind S: the installed web folder is found"
    );

    let r = get(&d, "remember-me");
    let srcs: Vec<Source> = r.installs.iter().map(|i| i.source).collect();
    assert_eq!(srcs, [Source::Store, Source::Nsis], "both copies are shown");
    assert_eq!(
        r.installs[1].size_bytes,
        Some(328_105 * 1024),
        "string DWORD is read"
    );
    assert_eq!(
        r.installs[1].location, "C:\\Users\\S\\AppData\\Local\\Programs\\Remember Me",
        "from UninstallString"
    );

    assert!(get(&d, "lemon").installs.is_empty());
}

#[test]
fn traces_without_a_copy() {
    let d = run("detect-traces-only.json", true);
    let f = get(&d, "instafind");
    assert!(f.installs.is_empty());
    assert!(f.used_before);

    let w = get(&d, "instawords");
    assert_eq!(w.installs.len(), 1);
    assert!(
        !w.installs[0].launchable,
        "record without its exe cannot be launched"
    );
    assert_eq!(w.installs[0].exe, None);
    assert_eq!(w.installs[0].version.as_deref(), Some("2.2.0"));

    assert!(get(&d, "instaconvert").installs.is_empty());
}

#[test]
fn empty_pc() {
    let view = FixtureView::from_json("{}").unwrap();
    let cat = catalog::parse(catalog::FIXTURE).unwrap();
    for a in detect::detect_all(&view, &catalog::visible(&cat, true)) {
        assert!(a.installs.is_empty() && !a.used_before, "{}", a.id);
    }
}

#[test]
fn fixture_format_is_strict() {
    assert!(FixtureView::from_json("{\"registry\": {\"HKXX\\\\a\": {}}}").is_err());
    assert!(FixtureView::from_json("{\"surprise\": 1}").is_err());
}

#[test]
fn command_lines() {
    assert_eq!(
        detect::command_exe("\"C:\\A B\\bahth.exe\" --startup").as_deref(),
        Some("C:\\A B\\bahth.exe")
    );
    assert_eq!(
        detect::command_exe("C:\\x\\a.exe /S").as_deref(),
        Some("C:\\x\\a.exe")
    );
    assert_eq!(
        detect::command_exe("C:\\x\\A.EXE").as_deref(),
        Some("C:\\x\\A.EXE")
    );
    assert_eq!(detect::command_exe("rundll32 x.dll"), None);
    assert_eq!(detect::command_exe("\"relative\\a.exe\""), None);
    assert_eq!(detect::command_exe("\"C:\\unterminated.exe"), None);
}
