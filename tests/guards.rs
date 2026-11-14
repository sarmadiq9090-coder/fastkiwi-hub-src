//! Static guards for the Hub's hard rules: detection never writes; the Hub has no network,
//! install or uninstall code of its own; updates and hosting go only through the apps' own
//! hub commands (src/hostproc.rs). A grep over the sources, so a later change that
//! adds such a call fails here first.

use std::path::Path;

fn sources() -> Vec<(String, String)> {
    let mut out = Vec::new();
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut stack = vec![
        root.join("src"),
        root.join("ui").join("js"),
        root.join("ui").join("stub-module"),
        // The embed adapter every app ships: no network, no storage, no host objects.
        root.join("sdk"),
    ];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if p
                .extension()
                .is_some_and(|x| x == "rs" || x == "js" || x == "html")
            {
                out.push((
                    p.display().to_string(),
                    std::fs::read_to_string(&p).unwrap(),
                ));
            }
        }
    }
    out
}

#[test]
fn no_registry_or_package_writes() {
    let forbidden = [
        "RegSetValue",
        "RegCreateKey",
        "RegDeleteKey",
        "RegDeleteValue",
        "RegDeleteTree",
        "RegSetKeyValue",
        "KEY_WRITE",
        "KEY_SET_VALUE",
        "KEY_ALL_ACCESS",
        "KEY_CREATE_SUB_KEY",
        "AddPackageAsync",
        "RemovePackageAsync",
        "RegisterPackageAsync",
        "StagePackageAsync",
        "DeprovisionPackage",
        "RequestDownloadAndInstall",
    ];
    for (path, text) in sources() {
        for f in forbidden {
            assert!(!text.contains(f), "{path} contains {f}");
        }
    }
}

#[test]
fn no_network_code() {
    let forbidden = [
        "std::net",
        "TcpStream",
        "TcpListener",
        "UdpSocket",
        "reqwest",
        "ureq",
        "hyper",
        "WinHttp",
        "URLDownloadToFile",
        "InternetOpen",
        "fetch(",
        "XMLHttpRequest",
        "WebSocket",
        "EventSource",
        "sendBeacon",
        "navigator.serviceWorker",
    ];
    for (path, text) in sources() {
        if path.ends_with("guards.rs") {
            continue;
        }
        for f in forbidden {
            assert!(!text.contains(f), "{path} contains {f}");
        }
    }
}

#[test]
fn launch_is_shell_execute_only() {
    // Standalone launches are ShellExecute and forget. The only process the Hub starts with
    // pipes is an app's hub command, in src/hostproc.rs (fastkiwi-hub-cli@1, D-H23).
    for (path, text) in sources() {
        for f in [
            "CreateProcess",
            "WinExec",
            "AssignProcessToJobObject",
            "ShellExecuteEx",
        ] {
            assert!(!text.contains(f), "{path} contains {f}");
        }
        if !path.ends_with("hostproc.rs") {
            for f in ["std::process::Command", "Command::new", "process::Stdio"] {
                assert!(!text.contains(f), "{path} contains {f}");
            }
        }
    }
}

#[test]
fn hub_commands_only_with_contract_flags() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let text = std::fs::read_to_string(root.join("src").join("hostproc.rs")).unwrap();
    // One place builds the command: the exe detection found, stdio piped, stderr to NUL, no
    // console window.
    assert_eq!(text.matches("Command::new(").count(), 1);
    assert_eq!(text.matches(".args(").count(), 1);
    assert!(text.contains(".stderr(Stdio::null())"));
    assert!(text.contains("CREATE_NO_WINDOW"));
    // Its arguments come only from the contract's own flags.
    assert!(text.contains("command(exe, &[hubcli::Command::HubHost.flag()])"));
    assert!(text.contains("let mut a = vec![hubcli::Command::HubUpdate.flag().to_string()];"));
    for flag in ["--export-data", "--hub-host\"", "--hub-update\""] {
        assert!(!text.contains(flag), "hostproc.rs spells {flag} by hand");
    }
    // Nobody else builds commands.
    for (path, t) in sources() {
        if !path.ends_with("hostproc.rs") {
            assert!(!t.contains("hostproc::command"), "{path}");
        }
    }
}

#[test]
fn module_pages_get_no_window_ipc() {
    // wry's `.with_ipc_handler` defines a frozen `window.ipc` in the page. Only the Hub's own
    // page may have it; module WebViews take web messages from WebView2 directly (D-H28).
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let main = std::fs::read_to_string(root.join("src").join("main.rs")).unwrap();
    assert_eq!(main.matches(".with_ipc_handler(").count(), 1);
    let hub_view = main
        .find("let hub = WebViewBuilder::new_with_web_context(&mut hub_ctx)")
        .unwrap();
    let ipc = main.find(".with_ipc_handler(").unwrap();
    assert!(ipc > hub_view, "the only IPC handler is the Hub page's own");
    assert!(main.contains("on_web_message(&webview,"));
}
