//! Navigation allowlist and the file-serving path guard.

use kiwi_hub::{allowlist, serve};

#[test]
fn module_stays_on_its_origin() {
    for ok in [
        "https://fkmod.instawords/index.html",
        "https://fkmod.instawords/assets/app.js?v=1#top",
        "https://FKMOD.InstaWords/",
        "fkmod://instawords/index.html",
        "about:blank",
    ] {
        assert!(allowlist::module_nav_allowed(ok, "instawords"), "{ok}");
    }
    for bad in [
        "https://fastkiwi.app/",
        "https://fkmod.instawords.evil.example/",
        "https://evil.example/fkmod.instawords/",
        "https://fkmod.instawords@evil.example/",
        "https://user:pw@fkmod.instawords/",
        "http://fkmod.instawords/index.html",
        "https://fkmod.instawords:8443/",
        "https://fkmod.lemon/index.html",
        "https://fastkiwi.hub/index.html",
        "fkmod://lemon/index.html",
        "fastkiwi://hub/index.html",
        "file:///C:/Windows/win.ini",
        "data:text/html,<script>alert(1)</script>",
        "javascript:alert(1)",
        "blob:https://fkmod.instawords/1234",
        "ms-windows-store://pdp/?productid=9NGB1F7T7JGN",
        "\\\\server\\share\\x.html",
        "",
    ] {
        assert!(!allowlist::module_nav_allowed(bad, "instawords"), "{bad}");
    }
    assert!(!allowlist::module_nav_allowed(
        "https://fkmod.x/",
        "bad id!"
    ));
}

#[test]
fn hub_stays_on_its_origin() {
    assert!(allowlist::hub_nav_allowed(
        "https://fastkiwi.hub/index.html"
    ));
    assert!(allowlist::hub_nav_allowed("fastkiwi://hub/index.html"));
    assert!(!allowlist::hub_nav_allowed(
        "https://fastkiwi.hub.evil.example/"
    ));
    assert!(!allowlist::hub_nav_allowed("https://fkmod.instawords/"));
    assert!(!allowlist::hub_nav_allowed("https://fastkiwi.app/"));
    assert!(!allowlist::hub_nav_allowed("http://fastkiwi.hub/"));
}

#[test]
fn start_urls_are_allowed() {
    assert!(allowlist::hub_nav_allowed(&allowlist::hub_start_url()));
    assert!(allowlist::module_nav_allowed(
        &allowlist::module_start_url("instawords", "index.html"),
        "instawords"
    ));
}

#[test]
fn request_paths_are_cleaned() {
    assert_eq!(
        serve::clean_rel_path("/", "index.html").as_deref(),
        Some("index.html")
    );
    assert_eq!(
        serve::clean_rel_path("/assets/app.js?x=1", "i").as_deref(),
        Some("assets/app.js")
    );
    assert_eq!(
        serve::clean_rel_path("/fonts/a%20b.woff2", "i").as_deref(),
        Some("fonts/a b.woff2")
    );
    for bad in [
        "/../secret",
        "/a/../../b",
        "/%2e%2e/x",
        "/a%5c..%5cb",
        "/C:/x",
        "/a//b",
        "/a/./b",
        "/x%00.js",
        "/x.",
        "/%zz",
    ] {
        assert_eq!(serve::clean_rel_path(bad, "i"), None, "{bad}");
    }
}

#[test]
fn files_cannot_escape_the_module_root() {
    let base = std::env::temp_dir().join(format!("kiwi-hub-test-{}", std::process::id()));
    let root = base.join("web");
    std::fs::create_dir_all(root.join("assets")).unwrap();
    std::fs::write(root.join("index.html"), "ok").unwrap();
    std::fs::write(root.join("assets").join("a.js"), "ok").unwrap();
    std::fs::write(base.join("secret.txt"), "no").unwrap();
    assert!(serve::resolve_in_root(&root, "index.html").is_some());
    assert!(serve::resolve_in_root(&root, "assets/a.js").is_some());
    assert!(
        serve::resolve_in_root(&root, "assets").is_none(),
        "folders are not served"
    );
    assert!(serve::resolve_in_root(&root, "../secret.txt").is_none());
    assert!(serve::resolve_in_root(&root, "missing.html").is_none());
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn csp_keeps_everything_on_its_own_origin() {
    for csp in [serve::HUB_CSP, serve::MODULE_CSP] {
        assert!(csp.contains("connect-src 'self'"));
        assert!(csp.contains("frame-src 'none'"));
        assert!(csp.contains("worker-src 'none'"));
        assert!(csp.contains("object-src 'none'"));
        assert!(!csp.contains("unsafe-eval"));
        assert!(!csp.contains("http:") && !csp.contains("https:") && !csp.contains('*'));
    }
}

#[test]
fn embedded_ui_has_its_pages() {
    for f in [
        "index.html",
        "js/main.js",
        "js/i18n.js",
        "css/base.css",
        "stub-module/index.html",
        "stub-module/module.js",
    ] {
        assert!(serve::ui_file(f).is_some(), "{f}");
    }
    assert!(serve::stub_file("index.html").is_some());
    assert!(serve::stub_file("hub-assets/css/base.css").is_some());
    assert!(
        serve::stub_file("hub-assets/js/main.js").is_none(),
        "the stub sees only fonts and base.css"
    );
    assert_eq!(serve::mime("a/b.WOFF2"), "font/woff2");
    assert_eq!(serve::mime("x.exe"), "application/octet-stream");
}

#[test]
fn process_module_stays_on_its_port() {
    use kiwi_hub::allowlist::{process_nav_allowed as ok, process_origin};
    assert!(ok("http://127.0.0.1:49731/", 49731));
    assert!(ok("http://127.0.0.1:49731/__fastkiwi/enter?key=abc", 49731));
    assert!(ok("http://127.0.0.1:49731/chat#x", 49731));
    assert!(ok("about:blank", 49731));
    for bad in [
        "http://127.0.0.1:49732/",
        "http://127.0.0.1/",
        "https://127.0.0.1:49731/",
        "http://localhost:49731/",
        "http://[::1]:49731/",
        "http://user@127.0.0.1:49731/",
        "http://127.0.0.1.evil.example:49731/",
        "http://evil.example/",
        "ws://127.0.0.1:49731/",
        "file:///C:/x",
        "data:text/html,x",
        "javascript:alert(1)",
        "fkmod://lemon/index.html",
    ] {
        assert!(!ok(bad, 49731), "{bad}");
    }
    assert_eq!(process_origin(49731), "http://127.0.0.1:49731/");
}

#[cfg(windows)]
#[test]
fn launch_targets_are_checked() {
    use kiwi_hub::win::{Target, launch, valid_aumid};
    assert!(valid_aumid(
        "FastKiwi.RememberMeReminders_sm1xjmj6v76yc!RememberMe"
    ));
    assert!(!valid_aumid("FastKiwi.RememberMeReminders_sm1xjmj6v76yc"));
    assert!(!valid_aumid("x!y"));
    assert!(!valid_aumid("FastKiwi.InstaFind_sm1xjmj6v76yc!a b"));
    // Refused before ShellExecute is reached: nothing starts.
    assert!(launch(&Target::Exe("relative.exe".into())).is_err());
    assert!(launch(&Target::Exe(r"C:\Windows\win.ini".into())).is_err());
    assert!(launch(&Target::Exe(r"C:\no\such\app.exe".into())).is_err());
    assert!(launch(&Target::Aumid("bad".into())).is_err());
}
