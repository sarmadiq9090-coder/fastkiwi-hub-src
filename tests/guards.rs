//! Static guards for the prototype's hard rules: detection never writes, and the Hub has no
//! network, install, update or uninstall code. A grep over the sources, so a later change that
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
    for (path, text) in sources() {
        for f in [
            "CreateProcess",
            "std::process::Command",
            "Command::new",
            "WinExec",
            "AssignProcessToJobObject",
        ] {
            assert!(!text.contains(f), "{path} contains {f}");
        }
    }
}
