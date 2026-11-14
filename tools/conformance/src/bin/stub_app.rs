//! `stub-app`: a small app that speaks fastkiwi-hub-cli@1 correctly (docs/HUB-APP-CONTRACT.md).
//!
//! It is the reference the app sessions can read, the target of the harness's own tests, and
//! (Phase D step 2) the process module the Hub's host-side tests launch. Std only, plus the
//! shared `embed.js`.
//!
//! Its app id is the `app` of the `fastkiwi-hub.json` next to it (default `stub`). Its "data"
//! is the folder `stub-data` next to it. A file `stub-fault.txt` next to it can name faults
//! (comma-separated) that break one rule each, so the tests can prove the harness notices.

use sha2::{Digest, Sha256};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

const VERSION: &str = "0.1.0";
const NEXT_VERSION: &str = "0.2.0";
const PROTO: &str = hubcli::PROTO;
const EMBED_JS: &str = include_str!("../../../../sdk/embed.js");

const PAGE_CSP: &str = "default-src 'self'; connect-src 'self'; img-src 'self' data:; \
style-src 'self'; script-src 'self'; frame-src 'none'; worker-src 'none'; object-src 'none'; \
form-action 'none'; base-uri 'none'; frame-ancestors 'none'";

fn exe_dir() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(Path::to_path_buf))
        .unwrap_or_default()
}

struct Faults(Vec<String>);

impl Faults {
    fn load() -> Faults {
        let text = std::fs::read_to_string(exe_dir().join("stub-fault.txt")).unwrap_or_default();
        Faults(
            text.split([',', '\n', '\r', ' '])
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .collect(),
        )
    }
    fn has(&self, f: &str) -> bool {
        self.0.iter().any(|x| x == f)
    }
}

fn app_id() -> String {
    std::fs::read_to_string(exe_dir().join(hubcli::APP_MANIFEST))
        .ok()
        .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
        .and_then(|v| v.get("app").and_then(|a| a.as_str()).map(str::to_string))
        .unwrap_or_else(|| "stub".into())
}

fn step() -> Duration {
    Duration::from_millis(
        std::env::var("FK_STUB_STEP_MS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(30),
    )
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let a: Vec<&str> = args.iter().map(String::as_str).collect();
    let faults = Faults::load();
    let code = match a.as_slice() {
        ["--hub-host"] => host(&faults),
        ["--hub-update", rest @ ..] => match update_args(rest) {
            Some(check_only) => update(check_only, &faults),
            None => hubcli::exit::USAGE,
        },
        ["--export-data", dir] => export(Path::new(dir), &faults),
        [] => {
            eprintln!("stub-app {VERSION}: no standalone window in this stub");
            0
        }
        _ => hubcli::exit::USAGE,
    };
    std::process::exit(code);
}

/// `--hub-update [--check-only] [--hub-owner <hwnd>]` → Some(check_only).
fn update_args(rest: &[&str]) -> Option<bool> {
    let mut check_only = false;
    let mut i = 0;
    while i < rest.len() {
        match rest[i] {
            "--check-only" if !check_only => check_only = true,
            "--hub-owner" => {
                rest.get(i + 1)?.parse::<u64>().ok()?;
                i += 1;
            }
            _ => return None,
        }
        i += 1;
    }
    Some(check_only)
}

fn random_b64(n: usize) -> String {
    let mut b = vec![0u8; n];
    getrandom::fill(&mut b).expect("OS random numbers");
    b64url(&b)
}

fn b64url(b: &[u8]) -> String {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut s = String::new();
    for c in b.chunks(3) {
        let n = (u32::from(c[0]) << 16)
            | (u32::from(*c.get(1).unwrap_or(&0)) << 8)
            | u32::from(*c.get(2).unwrap_or(&0));
        let chars = c.len() + 1;
        for i in 0..chars {
            s.push(A[((n >> (18 - 6 * i)) & 63) as usize] as char);
        }
    }
    s
}

fn same(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

fn out_line(s: &str) {
    let mut o = std::io::stdout().lock();
    let _ = writeln!(o, "{s}");
    let _ = o.flush();
}

// ---------------------------------------------------------------------------------------------
// --hub-host

struct Server {
    port: u16,
    key: String,
    session: String,
    app: String,
    any_host: bool,
    any_origin: bool,
    no_key_check: bool,
    cors_wildcard: bool,
    no_csp: bool,
    embed_modified: bool,
}

fn host(f: &Faults) -> i32 {
    let bind = if f.has("bind-all") {
        "0.0.0.0:0"
    } else {
        "127.0.0.1:0"
    };
    let Ok(listener) = TcpListener::bind(bind) else {
        return hubcli::exit::FAILED;
    };
    let port = listener.local_addr().map(|a| a.port()).unwrap_or(0);
    let key = if f.has("weak-key") {
        random_b64(12)
    } else {
        random_b64(32)
    };
    let srv = Arc::new(Server {
        port,
        key,
        session: random_b64(32),
        app: app_id(),
        any_host: f.has("any-host"),
        any_origin: f.has("any-origin"),
        no_key_check: f.has("no-key-check"),
        cors_wildcard: f.has("cors-wildcard"),
        no_csp: f.has("no-csp"),
        embed_modified: f.has("embed-modified"),
    });
    let mut hs = serde_json::json!({
        "proto": PROTO, "port": port, "key": srv.key, "name": srv.app, "version": VERSION,
    });
    if f.has("bad-handshake") {
        hs["url"] = serde_json::json!(format!("http://127.0.0.1:{port}/"));
    }
    out_line(&hs.to_string());
    if f.has("extra-stdout") {
        out_line("{\"note\":\"listening\"}");
    }
    let ignore_stdin = f.has("ignore-stdin");
    std::thread::spawn(move || {
        let mut sink = Vec::new();
        let _ = std::io::stdin().lock().read_to_end(&mut sink);
        if !ignore_stdin {
            std::process::exit(0);
        }
    });
    for conn in listener.incoming().flatten() {
        let srv = srv.clone();
        std::thread::spawn(move || {
            let _ = serve(conn, &srv);
        });
    }
    0
}

struct Req {
    method: String,
    target: String,
    headers: Vec<(String, String)>,
}

impl Req {
    fn header(&self, n: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k == n)
            .map(|(_, v)| v.as_str())
    }
    fn cookie(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .filter(|(k, _)| k == "cookie")
            .flat_map(|(_, v)| v.split(';'))
            .find_map(|kv| {
                let (k, v) = kv.trim().split_once('=')?;
                (k == name).then_some(v)
            })
    }
}

fn read_req(s: &TcpStream) -> std::io::Result<Req> {
    let mut r = BufReader::new(s.take(16 * 1024));
    let mut line = String::new();
    r.read_line(&mut line)?;
    let mut parts = line.split_ascii_whitespace();
    let method = parts.next().unwrap_or("").to_string();
    let target = parts.next().unwrap_or("").to_string();
    let mut headers = Vec::new();
    loop {
        let mut l = String::new();
        if r.read_line(&mut l)? == 0 {
            break;
        }
        let l = l.trim_end();
        if l.is_empty() {
            break;
        }
        if let Some((k, v)) = l.split_once(':') {
            headers.push((k.trim().to_ascii_lowercase(), v.trim().to_string()));
        }
    }
    Ok(Req {
        method,
        target,
        headers,
    })
}

fn serve(mut s: TcpStream, srv: &Server) -> std::io::Result<()> {
    s.set_read_timeout(Some(Duration::from_secs(5)))?;
    let req = read_req(&s)?;
    let (status, extra, ctype, body) = route(&req, srv);
    let reason = match status {
        200 => "OK",
        303 => "See Other",
        400 => "Bad Request",
        403 => "Forbidden",
        404 => "Not Found",
        _ => "Method Not Allowed",
    };
    let mut head = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Length: {}\r\n",
        body.len()
    );
    head.push_str("X-Content-Type-Options: nosniff\r\nCache-Control: no-store\r\n");
    head.push_str("Referrer-Policy: no-referrer\r\nCross-Origin-Resource-Policy: same-origin\r\n");
    if !srv.no_csp {
        head.push_str(&format!("Content-Security-Policy: {PAGE_CSP}\r\n"));
    }
    if srv.cors_wildcard {
        head.push_str("Access-Control-Allow-Origin: *\r\n");
    }
    if let Some(t) = ctype {
        head.push_str(&format!("Content-Type: {t}\r\n"));
    }
    for (k, v) in extra {
        head.push_str(&format!("{k}: {v}\r\n"));
    }
    head.push_str("Connection: close\r\n\r\n");
    s.write_all(head.as_bytes())?;
    if req.method != "HEAD" {
        s.write_all(&body)?;
    }
    s.flush()
}

type Out = (
    u16,
    Vec<(&'static str, String)>,
    Option<&'static str>,
    Vec<u8>,
);

fn deny(status: u16) -> Out {
    (status, Vec::new(), None, Vec::new())
}

fn route(req: &Req, srv: &Server) -> Out {
    // 1. Host: 127.0.0.1:<port> (localhost:<port> is tolerated, as the contract allows).
    let Some(host) = req.header("host") else {
        return deny(400);
    };
    let good_hosts = [
        format!("127.0.0.1:{}", srv.port),
        format!("localhost:{}", srv.port),
    ];
    if !srv.any_host && !good_hosts.iter().any(|h| h == host) {
        return deny(403);
    }
    // 2. Origin and Sec-Fetch-Site.
    if !srv.any_origin {
        let own = [
            format!("http://127.0.0.1:{}", srv.port),
            format!("http://localhost:{}", srv.port),
        ];
        match req.header("origin") {
            Some(o) if !own.iter().any(|x| x == o) => return deny(403),
            None if !matches!(req.method.as_str(), "GET" | "HEAD") => return deny(403),
            _ => {}
        }
        if let Some(sfs) = req.header("sec-fetch-site")
            && sfs != "same-origin"
            && sfs != "none"
        {
            return deny(403);
        }
    }
    let (path, query) = req
        .target
        .split_once('?')
        .unwrap_or((req.target.as_str(), ""));
    // 3. The key, once, for the session cookie.
    if path == hubcli::ENTER_PATH {
        let key = query
            .split('&')
            .find_map(|kv| kv.strip_prefix("key="))
            .unwrap_or("");
        if req.method != "GET" || !same(key.as_bytes(), srv.key.as_bytes()) {
            return deny(403);
        }
        let cookie = format!(
            "{}={}; Path=/; HttpOnly; SameSite=Strict",
            hubcli::COOKIE,
            srv.session
        );
        return (
            303,
            vec![("Location", "/".into()), ("Set-Cookie", cookie)],
            None,
            Vec::new(),
        );
    }
    // 4. The cookie on every other request.
    let authed = req
        .cookie(hubcli::COOKIE)
        .is_some_and(|c| same(c.as_bytes(), srv.session.as_bytes()));
    if !srv.no_key_check && !authed {
        return deny(403);
    }
    if req.method != "GET" && req.method != "HEAD" {
        return deny(405);
    }
    match path {
        "/" | "/index.html" => (
            200,
            Vec::new(),
            Some("text/html; charset=utf-8"),
            page(&srv.app).into_bytes(),
        ),
        hubcli::EMBED_PATH => {
            let mut js = EMBED_JS.as_bytes().to_vec();
            if srv.embed_modified {
                js.extend_from_slice(b"\n// patched\n");
            }
            (200, Vec::new(), Some("text/javascript; charset=utf-8"), js)
        }
        "/stub.js" => (
            200,
            Vec::new(),
            Some("text/javascript; charset=utf-8"),
            STUB_JS.as_bytes().to_vec(),
        ),
        "/stub.css" => (
            200,
            Vec::new(),
            Some("text/css; charset=utf-8"),
            STUB_CSS.as_bytes().to_vec(),
        ),
        _ => deny(404),
    }
}

fn page(app: &str) -> String {
    format!(
        r#"<!doctype html>
<html lang="ar" dir="rtl" data-theme="light" data-tone="light">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<meta name="color-scheme" content="light dark">
<meta name="fastkiwi-module" content="{app}">
<meta name="fastkiwi-module-version" content="{VERSION}">
<title>stub-app</title>
<link rel="stylesheet" href="/stub.css">
<script type="module" src="/__fastkiwi/embed.js"></script>
<script type="module" src="/stub.js"></script>
</head>
<body>
<header class="brand" data-fk-brand>stub-app {VERSION} (شريط البرنامج: يُخفى داخل الهاب)</header>
<main>
<h1>وحدة تجريبية من نوع P</h1>
<p>تتحدث fastkiwi-hub-cli@1 وfastkiwi-module@1. ليست برنامجًا حقيقيًا.</p>
<dl>
<dt>داخل الهاب</dt><dd id="hosted">—</dd>
<dt>المظهر</dt><dd id="theme">—</dd>
<dt>اللغة</dt><dd id="lang">—</dd>
<dt>الأرقام</dt><dd id="digits">—</dd>
<dt>التحديث</dt><dd id="update">—</dd>
</dl>
<button id="own" type="button">افتح في نافذته</button>
</main>
</body>
</html>
"#
    )
}

const STUB_JS: &str = r#"import { isHosted, on, formatNumber, openOwnWindow } from "/__fastkiwi/embed.js";
const $ = (id) => document.getElementById(id);
$("hosted").textContent = isHosted() ? "نعم" : "لا";
on("settings", (s) => {
  $("theme").textContent = `${s.theme} (${s.tone})`;
  $("lang").textContent = `${s.lang} (${s.dir})`;
  $("digits").textContent = `${s.digits} · ${formatNumber(1234567.5)}`;
});
on("update", (u) => {
  $("update").textContent = u.state + (typeof u.progress === "number" ? ` ${formatNumber(u.progress, { style: "percent" })}` : "");
});
$("own").addEventListener("click", openOwnWindow);
"#;

const STUB_CSS: &str = "body{font-family:system-ui,sans-serif;margin:0;padding:16px}\
.brand{background:#2b7a0b;color:#fff;padding:8px 16px;margin:-16px -16px 16px}\
html[data-tone=dark] body{background:#1b1b1b;color:#eee}\
dt{font-weight:600}dd{margin:0 0 8px}";

// ---------------------------------------------------------------------------------------------
// --hub-update

fn line(state: &str, extra: &[(&str, serde_json::Value)]) -> String {
    let mut v = serde_json::json!({ "proto": PROTO, "state": state });
    for (k, x) in extra {
        v[*k] = x.clone();
    }
    v.to_string()
}

fn update(check_only: bool, f: &Faults) -> i32 {
    let cancelled = Arc::new(AtomicBool::new(false));
    {
        let c = cancelled.clone();
        std::thread::spawn(move || {
            let mut sink = Vec::new();
            let _ = std::io::stdin().lock().read_to_end(&mut sink);
            c.store(true, Ordering::SeqCst);
        });
    }
    let step = step();
    out_line(&line("checking", &[]));
    std::thread::sleep(step);
    if f.has("update-silent") {
        std::thread::sleep(Duration::from_secs(10));
    }
    if f.has("bad-progress") {
        out_line(&line("downloading", &[("progress", 1.5.into())]));
        return hubcli::exit::OK;
    }
    if f.has("update-offline") || f.has("update-exit-mismatch") {
        out_line(&line("error", &[("code", "offline".into())]));
        return if f.has("update-exit-mismatch") {
            hubcli::exit::OK
        } else {
            hubcli::exit::FAILED
        };
    }
    if f.has("update-none") {
        out_line(&line("none", &[]));
        return hubcli::exit::OK;
    }
    if check_only {
        out_line(&line("available", &[("version", NEXT_VERSION.into())]));
        return hubcli::exit::OK;
    }
    // Download 0 → 0.8, then install 0.8 → 1, as the Store reports it.
    for i in 0..=8 {
        if cancelled.load(Ordering::SeqCst) {
            out_line(&line("error", &[("code", "cancelled".into())]));
            return hubcli::exit::FAILED;
        }
        let p = f64::from(i) / 10.0;
        out_line(&line(
            "downloading",
            &[("progress", p.into()), ("version", NEXT_VERSION.into())],
        ));
        std::thread::sleep(step);
    }
    for i in 8..=10 {
        out_line(&line(
            "installing",
            &[("progress", (f64::from(i) / 10.0).into())],
        ));
        std::thread::sleep(step);
    }
    out_line(&line("done", &[("version", NEXT_VERSION.into())]));
    hubcli::exit::OK
}

// ---------------------------------------------------------------------------------------------
// --export-data

fn utc_now() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs()) as i64;
    let (days, rem) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    // Civil from days (H. Hinnant).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

fn stub_data() -> std::io::Result<PathBuf> {
    let d = exe_dir().join("stub-data");
    if !d.exists() {
        std::fs::create_dir_all(d.join("notes"))?;
        std::fs::write(d.join("notes").join("today.txt"), "مرحبا من stub-app\n")?;
        std::fs::write(d.join("settings.json"), "{\"theme\":\"kiwi\"}\n")?;
    }
    Ok(d)
}

fn export(dir: &Path, f: &Faults) -> i32 {
    let fail = |code: &str| {
        out_line(&line("error", &[("code", code.into())]));
        hubcli::exit::FAILED
    };
    if dir.exists() {
        match std::fs::read_dir(dir) {
            Ok(mut it) => {
                if it.next().is_some() {
                    return fail("not-empty");
                }
            }
            Err(_) => return fail("access-denied"),
        }
    }
    let run = || -> std::io::Result<(u64, u64)> {
        let src = stub_data()?;
        let data = dir.join(hubcli::EXPORT_DATA_DIR);
        std::fs::create_dir_all(&data)?;
        let mut files = Vec::new();
        let mut stack = vec![src.clone()];
        while let Some(d) = stack.pop() {
            for e in std::fs::read_dir(&d)? {
                let e = e?;
                if e.file_type()?.is_dir() {
                    stack.push(e.path());
                    continue;
                }
                let rel = e
                    .path()
                    .strip_prefix(&src)
                    .expect("under src")
                    .to_string_lossy()
                    .replace('\\', "/");
                let bytes = std::fs::read(e.path())?;
                let target = data.join(&rel);
                if let Some(p) = target.parent() {
                    std::fs::create_dir_all(p)?;
                }
                std::fs::write(&target, &bytes)?;
                let mut sha: String = Sha256::digest(&bytes)
                    .iter()
                    .map(|b| format!("{b:02x}"))
                    .collect();
                if f.has("export-bad-hash") {
                    sha = "0".repeat(64);
                }
                files.push(serde_json::json!({
                    "path": format!("data/{rel}"), "size": bytes.len(), "sha256": sha,
                }));
            }
        }
        if f.has("export-extra-file") {
            std::fs::write(data.join("unlisted.bin"), b"x")?;
        }
        let total: u64 = files.iter().map(|x| x["size"].as_u64().unwrap_or(0)).sum();
        let manifest = serde_json::json!({
            "proto": PROTO, "app": app_id(), "version": VERSION, "created": utc_now(), "files": files,
        });
        std::fs::write(
            dir.join(hubcli::EXPORT_MANIFEST),
            serde_json::to_string_pretty(&manifest)? + "\n",
        )?;
        Ok((files.len() as u64, total))
    };
    match run() {
        Ok((n, bytes)) => {
            out_line(
                &serde_json::json!({ "proto": PROTO, "state": "done", "files": n, "bytes": bytes })
                    .to_string(),
            );
            hubcli::exit::OK
        }
        Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => fail("access-denied"),
        Err(_) => fail("internal"),
    }
}
