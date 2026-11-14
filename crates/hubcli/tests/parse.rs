//! fastkiwi-hub-cli@1 parsing: every line kind, accepted and refused.

use hubcli::*;

const KEY: &str = "AbCdEfGhIjKlMnOpQrStUvWxYz0123456789-_AbCdE";

fn hs(body: &str) -> Result<Handshake, CliError> {
    parse_handshake(
        &format!("{{\"proto\":\"fastkiwi-hub-cli@1\",{body}}}"),
        "lemon",
    )
}

#[test]
fn handshake() {
    assert_eq!(KEY.len(), KEY_LEN);
    let h = hs(&format!(
        r#""port":49731,"key":"{KEY}","name":"lemon","version":"2.3.0""#
    ))
    .unwrap();
    assert_eq!(h.port, 49731);
    assert_eq!(
        h.enter_url(),
        format!("http://127.0.0.1:49731/__fastkiwi/enter?key={KEY}")
    );
    assert_eq!(h.origin(), "http://127.0.0.1:49731");
    // CRLF from a Windows console is fine.
    assert!(
        parse_handshake(
            &format!(
                "{{\"proto\":\"fastkiwi-hub-cli@1\",\"port\":49731,\"key\":\"{KEY}\",\"name\":\"lemon\",\"version\":\"2.3.0\"}}\r\n"
            ),
            "lemon"
        )
        .is_ok()
    );

    let bad = [
        format!(r#""port":49731,"key":"{KEY}","name":"lemon""#), // no version
        format!(r#""port":49731,"key":"{KEY}","name":"lemon","version":"2.3""#),
        format!(r#""port":80,"key":"{KEY}","name":"lemon","version":"2.3.0""#),
        format!(r#""port":"49731","key":"{KEY}","name":"lemon","version":"2.3.0""#),
        format!(r#""port":70000,"key":"{KEY}","name":"lemon","version":"2.3.0""#),
        format!(r#""port":49731.5,"key":"{KEY}","name":"lemon","version":"2.3.0""#),
        r#""port":49731,"key":"short","name":"lemon","version":"2.3.0""#.to_string(),
        format!(
            r#""port":49731,"key":"{}=","name":"lemon","version":"2.3.0""#,
            &KEY[..42]
        ),
        format!(r#""port":49731,"key":"{KEY}","name":"instafind","version":"2.3.0""#),
        format!(r#""port":49731,"key":"{KEY}","name":"lemon","version":"2.3.0","url":"/""#),
        format!(r#""port":49731,"key":"{KEY}","name":"lemon","version":null"#),
        format!(r#""port":49731,"port":49732,"key":"{KEY}","name":"lemon","version":"2.3.0""#),
    ];
    for b in bad {
        assert!(hs(&b).is_err(), "accepted {b}");
    }
    assert_eq!(
        parse_handshake(
            &format!(
                r#"{{"proto":"fastkiwi-hub-cli@2","port":49731,"key":"{KEY}","name":"lemon","version":"2.3.0"}}"#
            ),
            "lemon"
        ),
        Err(CliError::BadProto)
    );
    assert_eq!(
        parse_handshake("listening on 49731", "lemon"),
        Err(CliError::NotJson)
    );
    assert_eq!(parse_handshake("[1]", "lemon"), Err(CliError::NotObject));
    let long = format!(
        r#"{{"proto":"fastkiwi-hub-cli@1","port":49731,"key":"{KEY}","name":"lemon","version":"2.3.0-{}"}}"#,
        "a".repeat(1100)
    );
    assert!(matches!(
        parse_handshake(&long, "lemon"),
        Err(CliError::TooLong(_))
    ));
}

fn ul(body: &str) -> Result<UpdateLine, CliError> {
    parse_update_line(&format!("{{\"proto\":\"fastkiwi-hub-cli@1\",{body}}}"))
}

#[test]
fn update_lines() {
    for good in [
        r#""state":"checking""#,
        r#""state":"available","version":"2.4.0""#,
        r#""state":"downloading","progress":0"#,
        r#""state":"downloading","progress":0.45,"version":"2.4.0""#,
        r#""state":"installing","progress":1"#,
        r#""state":"done""#,
        r#""state":"done","version":"2.4.0""#,
        r#""state":"none""#,
        r#""state":"error","code":"offline""#,
        r#""state":"error","code":"disk-full""#,
        r#""state":"error","code":"cancelled""#,
    ] {
        assert!(ul(good).is_ok(), "refused {good}");
    }
    for bad in [
        r#""state":"downloading""#,
        r#""state":"downloading","progress":1.2"#,
        r#""state":"downloading","progress":-0.1"#,
        r#""state":"installing","progress":"50%""#,
        r#""state":"checking","progress":0"#,
        r#""state":"available""#,
        r#""state":"available","version":"next""#,
        r#""state":"none","code":"offline""#,
        r#""state":"error""#,
        r#""state":"error","code":"Network is unreachable""#,
        r#""state":"error","code":"offline","message":"no wifi""#,
        r#""state":"restart-needed""#,
        r#""state":"failed""#,
        r#""state":"done","progress":null"#,
    ] {
        assert!(ul(bad).is_err(), "accepted {bad}");
    }
}

fn seq(check_only: bool, states: &[&str]) -> Result<(), CliError> {
    let mut r = UpdateRun::new(check_only);
    for s in states {
        r.push(&ul(s).unwrap())?;
    }
    Ok(())
}

#[test]
fn update_order() {
    let c = r#""state":"checking""#;
    let d = |p: f64| format!(r#""state":"downloading","progress":{p}"#);
    let i = |p: f64| format!(r#""state":"installing","progress":{p}"#);
    let done = r#""state":"done""#;
    let av = r#""state":"available","version":"2.4.0""#;
    let none = r#""state":"none""#;
    let err = r#""state":"error","code":"offline""#;

    // Full update.
    assert!(seq(false, &[c, &d(0.0), &d(0.8), &i(0.8), &i(1.0), done]).is_ok());
    assert!(seq(false, &[c, &i(0.9), done]).is_ok());
    assert!(seq(false, &[c, none]).is_ok());
    assert!(seq(false, &[c, &d(0.3), err]).is_ok());
    assert!(seq(false, &[c, av]).is_err(), "available is check-only");
    assert!(seq(false, &[&d(0.1)]).is_err(), "must start with checking");
    assert!(seq(false, &[c, c]).is_err());
    assert!(
        seq(false, &[c, &d(0.5), &d(0.4)]).is_err(),
        "progress went back"
    );
    assert!(seq(false, &[c, &i(0.9), &d(0.95)]).is_err());
    assert!(seq(false, &[c, &d(0.5), none]).is_err());
    assert!(seq(false, &[c, done, done]).is_err(), "line after the end");

    // Check only.
    assert!(seq(true, &[c, av]).is_ok());
    assert!(seq(true, &[c, none]).is_ok());
    assert!(seq(true, &[c, err]).is_ok());
    assert!(seq(true, &[c, &d(0.1)]).is_err());
    assert!(seq(true, &[c, done]).is_err());
}

#[test]
fn update_exit_codes() {
    let mut r = UpdateRun::new(true);
    r.push(&ul(r#""state":"checking""#).unwrap()).unwrap();
    assert!(matches!(r.finish(Some(0)), Err(CliError::Sequence(_))));
    r.push(&ul(r#""state":"error","code":"offline""#).unwrap())
        .unwrap();
    assert!(r.finish(Some(1)).is_ok());
    assert_eq!(
        r.finish(Some(0)),
        Err(CliError::ExitCode {
            expected: 1,
            got: Some(0)
        })
    );
    assert!(r.finish(None).is_err());

    let mut r = UpdateRun::new(true);
    r.push(&ul(r#""state":"checking""#).unwrap()).unwrap();
    r.push(&ul(r#""state":"none""#).unwrap()).unwrap();
    assert_eq!(r.finish(Some(0)).unwrap().state, UpdateState::None);
    assert!(r.finish(Some(1)).is_err());
}

#[test]
fn app_manifest() {
    let m = parse_app_manifest(
        "\u{feff}{\"proto\":\"fastkiwi-hub-cli@1\",\"app\":\"lemon\",\"commands\":[\"hub-host\",\"hub-update\",\"export-data\"]}\r\n",
        Some("lemon"),
    )
    .unwrap();
    assert!(m.has(Command::HubHost) && m.has(Command::ExportData));
    assert_eq!(Command::HubUpdate.flag(), "--hub-update");
    for bad in [
        r#"{"proto":"fastkiwi-hub-cli@1","app":"lemon","commands":[]}"#,
        r#"{"proto":"fastkiwi-hub-cli@1","app":"lemon","commands":["hub-host","hub-host"]}"#,
        r#"{"proto":"fastkiwi-hub-cli@1","app":"lemon","commands":["install"]}"#,
        r#"{"proto":"fastkiwi-hub-cli@1","app":"Lemon","commands":["hub-host"]}"#,
        r#"{"proto":"fastkiwi-hub-cli@1","app":"lemon","commands":["hub-host"],"exe":"x.exe"}"#,
        r#"{"proto":"fastkiwi-hub-cli@1","app":"instafind","commands":["hub-host"]}"#,
        r#"{"app":"lemon","commands":["hub-host"]}"#,
    ] {
        assert!(
            parse_app_manifest(bad, Some("lemon")).is_err(),
            "accepted {bad}"
        );
    }
    assert!(
        parse_app_manifest(
            r#"{"proto":"fastkiwi-hub-cli@1","app":"instafind","commands":["hub-host"]}"#,
            None
        )
        .is_ok()
    );
}

#[test]
fn export_result() {
    let p = |b: &str| parse_export_result(&format!("{{\"proto\":\"fastkiwi-hub-cli@1\",{b}}}"));
    assert!(p(r#""state":"done","files":3,"bytes":1200"#).is_ok());
    assert!(p(r#""state":"done","files":0,"bytes":0"#).is_ok());
    assert!(p(r#""state":"error","code":"not-empty""#).is_ok());
    assert!(p(r#""state":"done""#).is_err());
    assert!(p(r#""state":"done","files":3,"bytes":1200,"code":"internal""#).is_err());
    assert!(p(r#""state":"error","code":"offline""#).is_err());
    assert!(p(r#""state":"error","files":1,"bytes":1,"code":"internal""#).is_err());
    assert!(p(r#""state":"done","files":-1,"bytes":0"#).is_err());
}

#[test]
fn export_paths() {
    for good in [
        "data/chats.db",
        "data/notes/2026/today.txt",
        "data/ملاحظات.txt",
        "data/a.b.c",
    ] {
        assert!(safe_export_path(good), "{good}");
    }
    for bad in [
        "chats.db",
        "data/",
        "data//x",
        "data/../x",
        "data/./x",
        "/data/x",
        "data\\x",
        "data/c:x",
        "data/x:stream",
        "data/NUL",
        "data/con.txt",
        "data/COM1",
        "data/x.",
        "data/x ",
        "data/a*b",
        "data/a\u{0}b",
    ] {
        assert!(!safe_export_path(bad), "{bad}");
    }
}

#[test]
fn export_manifest() {
    let sha = "a".repeat(64);
    let doc = |files: &str, created: &str| {
        format!(
            r#"{{"proto":"fastkiwi-hub-cli@1","app":"lemon","version":"2.3.0","created":"{created}","files":[{files}]}}"#
        )
    };
    let f = |p: &str| format!(r#"{{"path":"{p}","size":10,"sha256":"{sha}"}}"#);
    let ok = doc(
        &format!("{},{}", f("data/a"), f("data/b/c")),
        "2026-11-14T08:30:00Z",
    );
    assert_eq!(parse_export_manifest(&ok, "lemon").unwrap().files.len(), 2);
    assert!(parse_export_manifest(&doc("", "2026-11-14T08:30:00Z"), "lemon").is_ok());
    assert!(parse_export_manifest(&ok, "instafind").is_err());
    for bad in [
        doc(&f("data/a"), "2026-11-14 08:30:00"),
        doc(&f("data/a"), "2026-13-14T08:30:00Z"),
        doc(&f("data/a"), "2026-11-14T08:30:00+03:00"),
        doc(
            &format!("{},{}", f("data/A"), f("data/a")),
            "2026-11-14T08:30:00Z",
        ),
        doc(&f("../a"), "2026-11-14T08:30:00Z"),
        doc(
            r#"{"path":"data/a","size":10,"sha256":"ABC"}"#,
            "2026-11-14T08:30:00Z",
        ),
        doc(
            r#"{"path":"data/a","size":10,"sha256":null}"#,
            "2026-11-14T08:30:00Z",
        ),
        doc(
            &format!(r#"{{"path":"data/a","size":10,"sha256":"{sha}","mode":420}}"#),
            "2026-11-14T08:30:00Z",
        ),
    ] {
        assert!(
            parse_export_manifest(&bad, "lemon").is_err(),
            "accepted {bad}"
        );
    }
}

#[test]
fn small_rules() {
    assert!(valid_key(KEY));
    assert!(!valid_key(&KEY.replace('A', "+")));
    assert!(valid_app_id("remember-me"));
    assert!(!valid_app_id("Remember Me"));
    assert!(is_semver("1.0.0-rc.1+build.5"));
    assert!(!is_semver("01.0.0"));
    assert!(valid_utc_stamp("2026-11-14T23:59:60Z"));
    assert!(!valid_utc_stamp("2026-11-14T24:00:00Z"));
    assert_eq!(
        UpdateLine {
            progress: Some(0.5),
            ..UpdateLine::new(UpdateState::Downloading)
        }
        .to_line(),
        r#"{"proto":"fastkiwi-hub-cli@1","state":"downloading","progress":0.5}"#
    );
}
