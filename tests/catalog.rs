//! Catalog parsing and visibility (D-069: InstaConvert hidden unless --dev).

use kiwi_hub::catalog::{self, CatalogError, Kind, Probe};

fn fixture() -> catalog::Catalog {
    catalog::parse(catalog::FIXTURE).expect("fixture catalog parses")
}

/// Replace the first `from` in the fixture with `to` and parse.
fn mutated(from: &str, to: &str) -> Result<catalog::Catalog, CatalogError> {
    assert!(catalog::FIXTURE.contains(from), "fixture lacks {from}");
    catalog::parse(&catalog::FIXTURE.replacen(from, to, 1))
}

#[test]
fn fixture_lists_our_five_apps() {
    let c = fixture();
    let ids: Vec<&str> = c.apps.iter().map(|a| a.id.as_str()).collect();
    assert_eq!(
        ids,
        [
            "instafind",
            "instawords",
            "remember-me",
            "lemon",
            "instaconvert"
        ]
    );
    assert_eq!(c.schema, 1);
    assert!(c.serial >= 1);
}

#[test]
fn shipped_view_hides_unreleased_instaconvert() {
    let c = fixture();
    let shown: Vec<String> = catalog::visible(&c, false)
        .into_iter()
        .map(|a| a.id)
        .collect();
    assert_eq!(shown, ["instafind", "instawords", "remember-me", "lemon"]);
    let dev: Vec<String> = catalog::visible(&c, true)
        .into_iter()
        .map(|a| a.id)
        .collect();
    assert!(dev.contains(&"instaconvert".to_string()));
}

#[test]
fn unknown_status_is_hidden_even_in_dev() {
    let c = mutated("\"status\": \"coming-soon\"", "\"status\": \"withdrawn\"").unwrap();
    assert!(
        !catalog::visible(&c, true)
            .iter()
            .any(|a| a.id == "instaconvert")
    );
}

#[test]
fn kinds_and_probes_match_the_design() {
    let c = fixture();
    let hub = |id: &str| {
        c.apps
            .iter()
            .find(|a| a.id == id)
            .unwrap()
            .hub
            .clone()
            .unwrap()
    };
    assert_eq!(hub("instawords").kind, Kind::S);
    assert!(hub("instawords").module.is_some());
    assert_eq!(hub("remember-me").kind, Kind::L);
    assert_eq!(hub("lemon").kind, Kind::P);
    assert!(hub("remember-me").detect.contains(&Probe::Store {
        pfn: "FastKiwi.RememberMeReminders_sm1xjmj6v76yc".into(),
        app_id: "RememberMe".into()
    }));
    assert!(
        hub("instafind")
            .detect
            .iter()
            .any(|p| matches!(p, Probe::RegHint { key } if key == "HKCU\\Software\\BahthSari"))
    );
}

#[test]
fn rejects_bad_files() {
    assert!(matches!(
        catalog::parse("not json"),
        Err(CatalogError::Json(_))
    ));
    assert!(matches!(
        mutated("\"schema\": 1", "\"schema\": 2"),
        Err(CatalogError::Schema(2))
    ));
    assert!(matches!(
        mutated("\"serial\": 1", "\"serial\": 0"),
        Err(CatalogError::Serial)
    ));
    assert!(matches!(
        mutated("\"expires\": \"2027-05-14\"", "\"expires\": \"soon\""),
        Err(CatalogError::Expires(_))
    ));
    assert!(matches!(
        mutated("\"id\": \"lemon\"", "\"id\": \"instafind\""),
        Err(CatalogError::DuplicateId(_))
    ));
    assert!(matches!(
        mutated("\"id\": \"lemon\"", "\"id\": \"Lemon!\""),
        Err(CatalogError::BadId(_))
    ));
    assert!(matches!(
        mutated(
            "\"icon\": \"icons/lemon.png\"",
            "\"icon\": \"../secret.png\""
        ),
        Err(CatalogError::BadIcon(_))
    ));
}

#[test]
fn rejects_probes_that_point_elsewhere() {
    let cases = [
        // absolute paths, traversal, UNC and other variables are not allowed
        (
            "%LOCALAPPDATA%\\\\Programs\\\\LocalAiChat\\\\local-ai-chat.exe",
            "C:\\\\Windows\\\\System32\\\\cmd.exe",
        ),
        (
            "%LOCALAPPDATA%\\\\Programs\\\\LocalAiChat\\\\local-ai-chat.exe",
            "%LOCALAPPDATA%\\\\..\\\\..\\\\x.exe",
        ),
        (
            "%LOCALAPPDATA%\\\\Programs\\\\LocalAiChat\\\\local-ai-chat.exe",
            "\\\\\\\\server\\\\share\\\\x.exe",
        ),
        (
            "%LOCALAPPDATA%\\\\Programs\\\\LocalAiChat\\\\local-ai-chat.exe",
            "%TEMP%\\\\x.exe",
        ),
        (
            "%LOCALAPPDATA%\\\\Programs\\\\LocalAiChat\\\\local-ai-chat.exe",
            "%LOCALAPPDATA%\\\\Programs\\\\x.bat",
        ),
        // a Store PFN must look like Name_publisherid
        ("FastKiwi.InstaFind_sm1xjmj6v76yc", "FastKiwi.InstaFind"),
        // exe names are plain names
        (
            "\"exe\": \"Remember Me.exe\"",
            "\"exe\": \"..\\\\evil.exe\"",
        ),
        // hint keys only under HKCU\Software
        (
            "HKCU\\\\Software\\\\BahthSari",
            "HKLM\\\\SYSTEM\\\\CurrentControlSet",
        ),
        // unknown probe kinds and unknown fields
        ("\"kind\": \"run_value\"", "\"kind\": \"shell\""),
        (
            "\"kind\": \"run_value\", \"name\": \"BahthSari\"",
            "\"kind\": \"run_value\", \"name\": \"BahthSari\", \"cmd\": \"x\"",
        ),
    ];
    for (from, to) in cases {
        let r = mutated(from, to);
        assert!(r.is_err(), "{from} -> {to} should be refused, got {r:?}");
    }
}

#[test]
fn web_module_only_for_kind_s() {
    let r = mutated(
        "\"kind\": \"L\",",
        "\"kind\": \"L\", \"module\": { \"web_dir\": \"web\", \"entry\": \"index.html\" },",
    );
    assert!(matches!(r, Err(CatalogError::BadProbe { .. })), "{r:?}");
    let r = mutated("\"entry\": \"index.html\"", "\"entry\": \"../index.html\"");
    assert!(r.is_err());
}

#[test]
fn path_rules() {
    assert!(catalog::valid_catalog_path(
        "%LOCALAPPDATA%\\Programs\\InstaFind\\bahth.exe"
    ));
    assert!(catalog::valid_catalog_path("%localappdata%\\x"));
    assert!(!catalog::valid_catalog_path("%LOCALAPPDATA%"));
    assert!(!catalog::valid_catalog_path("%LOCALAPPDATA%\\a\\\\b"));
    assert!(!catalog::valid_catalog_path("%LOCALAPPDATA%\\a/b"));
    assert!(!catalog::valid_catalog_path("%LOCALAPPDATA%\\C:\\x"));
    assert!(catalog::valid_exe_name("Remember Me.exe"));
    assert!(!catalog::valid_exe_name("a.exe:stream"));
    assert!(!catalog::valid_exe_name(" a.exe"));
    assert!(catalog::valid_pfn("FastKiwi.InstaWords_sm1xjmj6v76yc"));
    assert!(!catalog::valid_pfn("FastKiwi.InstaWords_SM1XJMJ6V76YC"));
}

#[test]
fn updates_fixture_is_valid_json() {
    let v: serde_json::Value = serde_json::from_str(catalog::UPDATES_FIXTURE).unwrap();
    assert_eq!(v["rows"].as_array().unwrap().len(), 4);
}
