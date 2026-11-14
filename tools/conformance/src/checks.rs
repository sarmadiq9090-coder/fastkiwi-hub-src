//! The numbered checks. IDs are stable: the contract (§9) and the app sessions refer to them.

use crate::http::{self, Resp};
use crate::proc::{Line, Running};
use hubcli::{
    APP_MANIFEST, COOKIE, Command, EMBED_PATH, EMBED_RELEASES, ENTER_PATH, EXIT_GRACE,
    EXPORT_DATA_DIR, EXPORT_MANIFEST, ExportCode, ExportState, HANDSHAKE_TIMEOUT, Handshake,
    UPDATE_FIRST_LINE, UPDATE_SILENCE, UpdateRun, UpdateState, exit,
};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Pass(String),
    Fail(String),
    Skip(String),
}

#[derive(Debug, Clone)]
pub struct Item {
    pub id: &'static str,
    pub title: &'static str,
    pub outcome: Outcome,
}

#[derive(Debug, Clone, Default)]
pub struct Report {
    pub app: Option<String>,
    pub items: Vec<Item>,
}

impl Report {
    pub fn passed(&self) -> bool {
        !self
            .items
            .iter()
            .any(|i| matches!(i.outcome, Outcome::Fail(_)))
    }
    pub fn get(&self, id: &str) -> Option<&Item> {
        self.items.iter().find(|i| i.id == id)
    }
    pub fn failed(&self, id: &str) -> bool {
        self.get(id)
            .is_some_and(|i| matches!(i.outcome, Outcome::Fail(_)))
    }
    pub fn to_text(&self) -> String {
        let mut s = format!(
            "fastkiwi-hub-cli@1 conformance: {}\n",
            self.app.as_deref().unwrap_or("(unknown app)")
        );
        for i in &self.items {
            let (tag, note) = match &i.outcome {
                Outcome::Pass(n) => ("PASS", n),
                Outcome::Fail(n) => ("FAIL", n),
                Outcome::Skip(n) => ("SKIP", n),
            };
            s.push_str(&format!("{tag} {:<4} {}", i.id, i.title));
            if !note.is_empty() {
                s.push_str(&format!(": {note}"));
            }
            s.push('\n');
        }
        let fails = self
            .items
            .iter()
            .filter(|i| matches!(i.outcome, Outcome::Fail(_)))
            .count();
        s.push_str(&if fails == 0 {
            "RESULT conforms\n".to_string()
        } else {
            format!("RESULT {fails} check(s) failed\n")
        });
        s
    }
    fn add(&mut self, id: &'static str, outcome: Outcome) {
        let title = TITLES
            .iter()
            .find(|(i, _)| *i == id)
            .map_or("?", |(_, t)| t);
        self.items.push(Item { id, title, outcome });
    }
    fn skip_rest(&mut self, ids: &[&'static str], why: &str) {
        for id in ids {
            if self.get(id).is_none() {
                self.add(id, Outcome::Skip(why.into()));
            }
        }
    }
}

const TITLES: &[(&str, &str)] = &[
    ("M1", "fastkiwi-hub.json next to the exe"),
    ("H1", "--hub-host prints one valid handshake line"),
    ("H2", "listens on 127.0.0.1 only"),
    ("H3", "no cookie: 403 on every path"),
    ("H4", "wrong key: 403"),
    ("H5", "enter: 303 to / with the session cookie"),
    ("H6", "page: HTML, CSP, nosniff, module meta, embed.js"),
    ("H7", "embed.js is a pinned release"),
    ("H8", "Host guard"),
    ("H9", "Origin and Sec-Fetch-Site guard"),
    ("H10", "no CORS"),
    ("H11", "two runs at once: own port, own key"),
    ("H12", "no visible window"),
    ("H13", "nothing more on stdout"),
    ("H14", "exits when stdin closes"),
    (
        "U1",
        "--hub-update --check-only: valid lines, order, exit code",
    ),
    ("E1", "--export-data into a new folder"),
    ("E2", "--export-data refuses a non-empty folder"),
    ("C1", "bad command lines exit 2 and print nothing"),
];

const HOST_IDS: &[&str] = &[
    "H1", "H2", "H3", "H4", "H5", "H6", "H7", "H8", "H9", "H10", "H11", "H12", "H13", "H14",
];

#[derive(Debug, Clone)]
pub struct Options {
    /// The catalog id the app must report. Default: the id in its fastkiwi-hub.json.
    pub app: Option<String>,
    /// Longest silence allowed between two `--hub-update` lines.
    pub update_silence: Duration,
    /// Look for visible windows of the hub-host process (Windows only).
    pub check_windows: bool,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            app: None,
            update_silence: UPDATE_SILENCE,
            check_windows: cfg!(windows),
        }
    }
}

/// Run every check that applies to `exe`.
pub fn check_app(exe: &Path, opts: &Options) -> Report {
    let mut rep = Report::default();
    let manifest_path = exe
        .parent()
        .map(|d| d.join(APP_MANIFEST))
        .unwrap_or_default();
    let manifest = match std::fs::read_to_string(&manifest_path) {
        Err(e) => Err(format!("{}: {e}", manifest_path.display())),
        Ok(t) => hubcli::parse_app_manifest(&t, opts.app.as_deref()).map_err(|e| e.to_string()),
    };
    let manifest = match manifest {
        Ok(m) => {
            rep.add(
                "M1",
                Outcome::Pass(format!(
                    "{} declares {}",
                    m.app,
                    m.commands
                        .iter()
                        .map(|c| c.flag())
                        .collect::<Vec<_>>()
                        .join(" ")
                )),
            );
            m
        }
        Err(e) => {
            rep.add("M1", Outcome::Fail(e));
            let ids: Vec<&'static str> = TITLES.iter().skip(1).map(|(i, _)| *i).collect();
            rep.skip_rest(&ids, "needs M1");
            return rep;
        }
    };
    rep.app = Some(manifest.app.clone());
    let app = manifest.app.as_str();

    if manifest.has(Command::HubHost) {
        hub_host(exe, app, opts, &mut rep);
    } else {
        rep.skip_rest(HOST_IDS, "--hub-host not declared");
    }
    if manifest.has(Command::HubUpdate) {
        let o = hub_update(exe, opts);
        rep.add("U1", o);
    } else {
        rep.add("U1", Outcome::Skip("--hub-update not declared".into()));
    }
    if manifest.has(Command::ExportData) {
        export(exe, app, &mut rep);
    } else {
        rep.skip_rest(&["E1", "E2"], "--export-data not declared");
    }
    let o = usage(exe, &manifest.commands);
    rep.add("C1", o);
    rep
}

fn pass(s: impl Into<String>) -> Outcome {
    Outcome::Pass(s.into())
}
fn fail(s: impl Into<String>) -> Outcome {
    Outcome::Fail(s.into())
}

/// Start `--hub-host` and read its handshake.
fn start_host(exe: &Path, app: &str) -> Result<(Running, Handshake, Instant), String> {
    let r = Running::spawn(exe, &["--hub-host"]).map_err(|e| format!("cannot start: {e}"))?;
    let t0 = Instant::now();
    let line = match r.next_line(HANDSHAKE_TIMEOUT) {
        Line::Text(l) => l,
        Line::Eof => return Err("stdout closed before the handshake".into()),
        Line::Timeout => {
            return Err(format!(
                "no handshake within {} s",
                HANDSHAKE_TIMEOUT.as_secs()
            ));
        }
    };
    let text = String::from_utf8(line).map_err(|_| "handshake is not UTF-8".to_string())?;
    let h = hubcli::parse_handshake(&text, app).map_err(|e| e.to_string())?;
    Ok((r, h, t0))
}

struct Probe<'a> {
    h: &'a Handshake,
    host: String,
    seen: Vec<(String, Resp)>,
}

impl Probe<'_> {
    fn req(
        &mut self,
        what: &str,
        method: &str,
        path: &str,
        hdrs: &[(&str, &str)],
    ) -> Result<Resp, String> {
        let r =
            http::request(self.h.port, method, path, hdrs).map_err(|e| format!("{what}: {e}"))?;
        self.seen.push((what.to_string(), r.clone()));
        Ok(r)
    }
}

fn hub_host(exe: &Path, app: &str, opts: &Options, rep: &mut Report) {
    let (mut run, h, t0) = match start_host(exe, app) {
        Ok(x) => x,
        Err(e) => {
            rep.add("H1", fail(e));
            rep.skip_rest(HOST_IDS, "needs H1");
            return;
        }
    };
    rep.add(
        "H1",
        pass(format!(
            "port {}, version {}, {} ms",
            h.port,
            h.version,
            t0.elapsed().as_millis()
        )),
    );
    let mut p = Probe {
        h: &h,
        host: format!("127.0.0.1:{}", h.port),
        seen: Vec::new(),
    };

    rep.add("H2", loopback_only(h.port));

    // H3: nothing without the cookie.
    let host = p.host.clone();
    let o = (|| {
        for path in ["/", EMBED_PATH, "/api/x", "/index.html"] {
            let r = p.req("no cookie", "GET", path, &[("Host", &host)])?;
            if r.status != 403 {
                return Err(format!("GET {path} without the cookie gave {}", r.status));
            }
        }
        Ok(())
    })();
    rep.add("H3", o.map_or_else(fail, |_| pass("")));

    // H4: a wrong key opens nothing.
    let wrong = "A".repeat(hubcli::KEY_LEN);
    let o = (|| {
        for target in [
            format!("{ENTER_PATH}?key={wrong}"),
            ENTER_PATH.to_string(),
            format!("{ENTER_PATH}?key="),
            format!("{ENTER_PATH}?key={}", &h.key[..h.key.len() - 1]),
        ] {
            let r = p.req("wrong key", "GET", &target, &[("Host", &host)])?;
            if r.status != 403 {
                return Err(format!("GET {target} gave {}", r.status));
            }
            if r.header("set-cookie").is_some() {
                return Err(format!("GET {target} set a cookie"));
            }
        }
        Ok(())
    })();
    rep.add("H4", o.map_or_else(fail, |_| pass("")));

    // H5: the right key gives the session cookie.
    let enter = format!("{ENTER_PATH}?key={}", h.key);
    let cookie = match p
        .req("enter", "GET", &enter, &[("Host", &host)])
        .and_then(|r| check_enter(&r, h.port))
    {
        Ok(c) => {
            rep.add("H5", pass(""));
            c
        }
        Err(e) => {
            rep.add("H5", fail(e));
            rep.skip_rest(&["H6", "H7", "H8", "H9", "H10"], "needs H5");
            finish_host(&mut run, exe, app, &h, t0, opts, rep);
            return;
        }
    };
    let ck = format!("{COOKIE}={cookie}");
    let own_origin = h.origin();

    // H6: the page.
    let o = p
        .req("page", "GET", "/", &[("Host", &host), ("Cookie", &ck)])
        .and_then(|r| check_page(&r, app, h.port));
    rep.add("H6", o.map_or_else(fail, |_| pass("")));

    // H7: the adapter.
    let o = p
        .req(
            "embed",
            "GET",
            EMBED_PATH,
            &[("Host", &host), ("Cookie", &ck)],
        )
        .and_then(|r| check_embed(&r));
    rep.add("H7", o.map_or_else(fail, pass));

    // H8: Host guard.
    let other = format!(
        "127.0.0.1:{}",
        if h.port == 65535 { 1024 } else { h.port + 1 }
    );
    let o = (|| {
        for bad in [
            "evil.example".to_string(),
            format!("evil.example:{}", h.port),
            other.clone(),
            "127.0.0.1".to_string(),
            format!("[::1]:{}", h.port),
            format!("127.0.0.1:{}.evil.example", h.port),
        ] {
            let r = p.req("host", "GET", "/", &[("Host", &bad), ("Cookie", &ck)])?;
            if r.status != 403 && r.status != 400 && r.status != 421 {
                return Err(format!("Host: {bad} gave {}", r.status));
            }
        }
        let r = p.req("no host", "GET", "/", &[("Cookie", &ck)])?;
        if r.status != 403 && r.status != 400 {
            return Err(format!("no Host header gave {}", r.status));
        }
        Ok(())
    })();
    rep.add("H8", o.map_or_else(fail, |_| pass("")));

    // H9: Origin and Sec-Fetch-Site.
    let other_origin = format!("http://{other}");
    let o = (|| {
        let base = [("Host", host.as_str()), ("Cookie", ck.as_str())];
        let refused: [Refused; 7] = [
            (
                "GET",
                "foreign Origin",
                vec![("Origin", "http://evil.example")],
            ),
            ("GET", "Origin null", vec![("Origin", "null")]),
            (
                "GET",
                "another port's Origin",
                vec![("Origin", other_origin.as_str())],
            ),
            ("GET", "https Origin", vec![("Origin", "https://127.0.0.1")]),
            (
                "GET",
                "Sec-Fetch-Site cross-site",
                vec![("Sec-Fetch-Site", "cross-site")],
            ),
            (
                "GET",
                "Sec-Fetch-Site same-site",
                vec![("Sec-Fetch-Site", "same-site")],
            ),
            ("POST", "POST without Origin", vec![]),
        ];
        for (method, what, extra) in refused {
            let mut hd: Vec<(&str, &str)> = base.to_vec();
            hd.extend(extra);
            let r = p.req(what, method, "/", &hd)?;
            if r.status != 403 {
                return Err(format!("{what} gave {}", r.status));
            }
        }
        let mut hd: Vec<(&str, &str)> = base.to_vec();
        hd.push(("Origin", own_origin.as_str()));
        let r = p.req("own POST", "POST", "/", &hd)?;
        if r.status == 403 {
            return Err("POST with its own Origin was refused".into());
        }
        hd.push(("Sec-Fetch-Site", "same-origin"));
        let r = p.req("own GET", "GET", "/", &hd)?;
        if r.status != 200 {
            return Err(format!("GET with its own Origin gave {}", r.status));
        }
        Ok(())
    })();
    rep.add("H9", o.map_or_else(fail, |_| pass("")));

    // H10: no CORS anywhere.
    let o = (|| {
        let r = p.req(
            "preflight",
            "OPTIONS",
            "/",
            &[
                ("Host", host.as_str()),
                ("Origin", "http://evil.example"),
                ("Access-Control-Request-Method", "POST"),
            ],
        )?;
        if r.status != 403 {
            return Err(format!("a foreign preflight gave {}", r.status));
        }
        for (what, r) in &p.seen {
            if let Some(v) = r.header("access-control-allow-origin") {
                return Err(format!("{what}: Access-Control-Allow-Origin: {v}"));
            }
        }
        Ok(())
    })();
    rep.add("H10", o.map_or_else(fail, |_| pass("")));

    finish_host(&mut run, exe, app, &h, t0, opts, rep);
}

/// H11 to H14, which need only the running process.
fn finish_host(
    run: &mut Running,
    exe: &Path,
    app: &str,
    h: &Handshake,
    t0: Instant,
    opts: &Options,
    rep: &mut Report,
) {
    // H11: a second run beside the first.
    let o = match start_host(exe, app) {
        Err(e) => fail(format!("second run: {e}")),
        Ok((mut r2, h2, _)) => {
            let o = if h2.port == h.port {
                fail("same port twice")
            } else if h2.key == h.key {
                fail("same key twice")
            } else {
                match http::request(
                    h2.port,
                    "GET",
                    &format!("{ENTER_PATH}?key={}", h2.key),
                    &[("Host", &format!("127.0.0.1:{}", h2.port))],
                ) {
                    Ok(r) if r.status == 303 => match http::request(
                        h2.port,
                        "GET",
                        &format!("{ENTER_PATH}?key={}", h.key),
                        &[("Host", &format!("127.0.0.1:{}", h2.port))],
                    ) {
                        Ok(r) if r.status == 403 => pass(""),
                        Ok(r) => fail(format!(
                            "the first run's key opened the second ({})",
                            r.status
                        )),
                        Err(e) => fail(e.to_string()),
                    },
                    Ok(r) => fail(format!("second run's enter gave {}", r.status)),
                    Err(e) => fail(e.to_string()),
                }
            };
            r2.close_stdin();
            if r2
                .wait_timeout(EXIT_GRACE + Duration::from_millis(500))
                .is_none()
            {
                r2.kill();
            }
            o
        }
    };
    rep.add("H11", o);

    // H12: no window.
    if opts.check_windows && cfg!(windows) {
        let wait = Duration::from_millis(1500).saturating_sub(t0.elapsed());
        std::thread::sleep(wait);
        let n = visible_windows(run.pid());
        rep.add(
            "H12",
            if n == 0 {
                pass("")
            } else {
                fail(format!("{n} visible top-level window(s)"))
            },
        );
    } else {
        rep.add("H12", Outcome::Skip("not checked on this system".into()));
    }

    // H13 and H14.
    let early = run.pending_lines();
    run.close_stdin();
    let exited = run.wait_timeout(EXIT_GRACE + Duration::from_millis(500));
    let late = if exited.is_some() {
        // stdout closes when the process ends; collect what is left.
        let mut v = Vec::new();
        while let Line::Text(l) = run.next_line(Duration::from_millis(500)) {
            v.push(l);
        }
        v
    } else {
        run.pending_lines()
    };
    let extra = early.len() + late.len();
    rep.add(
        "H13",
        if extra == 0 {
            pass("")
        } else {
            fail(format!("{extra} more line(s) after the handshake"))
        },
    );
    rep.add(
        "H14",
        match exited {
            None => {
                run.kill();
                fail(format!(
                    "still running {} ms after stdin closed (killed)",
                    (EXIT_GRACE + Duration::from_millis(500)).as_millis()
                ))
            }
            Some(s) if s.code() == Some(exit::OK) => pass(""),
            Some(s) => fail(format!("exit code {:?}, expected 0", s.code())),
        },
    );
}

fn loopback_only(port: u16) -> Outcome {
    use std::net::{IpAddr, SocketAddr, TcpStream, UdpSocket};
    let mut tried = Vec::new();
    // [::1] must not answer: the contract binds 127.0.0.1, not "localhost".
    let v6 = SocketAddr::from((std::net::Ipv6Addr::LOCALHOST, port));
    if TcpStream::connect_timeout(&v6, Duration::from_millis(800)).is_ok() {
        return fail(format!("also listens on {v6}"));
    }
    tried.push(v6.to_string());
    // A routed local address. UDP connect sends no packet; it only picks the interface.
    let local = UdpSocket::bind("0.0.0.0:0")
        .and_then(|s| s.connect("192.0.2.1:9").map(|_| s))
        .and_then(|s| s.local_addr())
        .ok()
        .map(|a| a.ip())
        .filter(|ip| !ip.is_loopback() && !ip.is_unspecified());
    if let Some(ip @ IpAddr::V4(_)) = local {
        let a = SocketAddr::new(ip, port);
        if TcpStream::connect_timeout(&a, Duration::from_millis(800)).is_ok() {
            return fail(format!("also listens on {a}"));
        }
        tried.push(a.to_string());
    }
    pass(format!("refused on {}", tried.join(", ")))
}

fn check_enter(r: &Resp, port: u16) -> Result<String, String> {
    if r.status != 303 {
        return Err(format!("status {}, expected 303", r.status));
    }
    let loc = r.header("location").unwrap_or("");
    if loc != "/" && loc != format!("http://127.0.0.1:{port}/") {
        return Err(format!("Location {loc:?}, expected \"/\""));
    }
    let cookies = r.all("set-cookie");
    let c = cookies
        .iter()
        .find(|c| c.starts_with(&format!("{COOKIE}=")))
        .ok_or_else(|| format!("no Set-Cookie {COOKIE}"))?;
    let mut parts = c.split(';').map(str::trim);
    let value = parts
        .next()
        .and_then(|kv| kv.split_once('='))
        .map(|(_, v)| v.to_string())
        .unwrap_or_default();
    if value.len() < 22
        || !value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        return Err("cookie value must be at least 22 characters of [A-Za-z0-9_-]".into());
    }
    let attrs: Vec<String> = parts.map(|a| a.to_ascii_lowercase()).collect();
    for need in ["httponly", "samesite=strict", "path=/"] {
        if !attrs.iter().any(|a| a.replace(' ', "") == need) {
            return Err(format!("cookie lacks {need}"));
        }
    }
    if let Some(a) = attrs
        .iter()
        .find(|a| a.starts_with("domain") || a.starts_with("expires") || a.starts_with("max-age"))
    {
        return Err(format!(
            "cookie must be a host-only session cookie (has {a})"
        ));
    }
    let cc = r.header("cache-control").unwrap_or("").to_ascii_lowercase();
    if !cc.contains("no-store") {
        return Err("enter response lacks Cache-Control: no-store".into());
    }
    let rp = r
        .header("referrer-policy")
        .unwrap_or("")
        .to_ascii_lowercase();
    if rp != "no-referrer" {
        return Err("enter response lacks Referrer-Policy: no-referrer".into());
    }
    Ok(value)
}

/// The sources the page may connect to, from its CSP (connect-src, else default-src).
fn connect_sources(csp: &str) -> Option<Vec<String>> {
    let dirs: BTreeMap<String, Vec<String>> = csp
        .split(';')
        .filter_map(|d| {
            let mut w = d.split_ascii_whitespace();
            let name = w.next()?.to_ascii_lowercase();
            Some((name, w.map(str::to_string).collect()))
        })
        .collect();
    dirs.get("connect-src")
        .or_else(|| dirs.get("default-src"))
        .cloned()
}

fn check_page(r: &Resp, app: &str, port: u16) -> Result<(), String> {
    if r.status != 200 {
        return Err(format!("GET / gave {}", r.status));
    }
    let ct = r.header("content-type").unwrap_or("").to_ascii_lowercase();
    if !ct.starts_with("text/html") {
        return Err(format!("Content-Type {ct:?}"));
    }
    if r.header("x-content-type-options")
        .is_none_or(|v| !v.eq_ignore_ascii_case("nosniff"))
    {
        return Err("no X-Content-Type-Options: nosniff".into());
    }
    let allowed = [
        "'self'".to_string(),
        "'none'".to_string(),
        format!("http://127.0.0.1:{port}"),
        format!("ws://127.0.0.1:{port}"),
    ];
    let policies = r.all("content-security-policy");
    if policies.is_empty() {
        return Err("no Content-Security-Policy header".into());
    }
    let ok = policies.iter().any(|p| {
        connect_sources(p).is_some_and(|s| !s.is_empty() && s.iter().all(|x| allowed.contains(x)))
    });
    if !ok {
        return Err("CSP connect-src (or default-src) must allow only 'self'".into());
    }
    let html = String::from_utf8_lossy(&r.body);
    let metas = tags(&html, "meta");
    let has_meta = |name: &str, check: &dyn Fn(&str) -> bool| {
        metas.iter().any(|m| {
            attr(m, "name").as_deref() == Some(name)
                && attr(m, "content").is_some_and(|c| check(&c))
        })
    };
    if !has_meta("fastkiwi-module", &|c| c == app) {
        return Err(format!(
            "no <meta name=\"fastkiwi-module\" content=\"{app}\">"
        ));
    }
    if !has_meta("fastkiwi-module-version", &hubcli::is_semver) {
        return Err("no <meta name=\"fastkiwi-module-version\"> with a SemVer version".into());
    }
    let script = tags(&html, "script").iter().any(|s| {
        attr(s, "type").as_deref() == Some("module")
            && attr(s, "src").as_deref() == Some(EMBED_PATH)
    });
    if !script {
        return Err(format!("no <script type=\"module\" src=\"{EMBED_PATH}\">"));
    }
    Ok(())
}

fn check_embed(r: &Resp) -> Result<String, String> {
    if r.status != 200 {
        return Err(format!("GET {EMBED_PATH} gave {}", r.status));
    }
    let ct = r.header("content-type").unwrap_or("").to_ascii_lowercase();
    if !ct.starts_with("text/javascript") && !ct.starts_with("application/javascript") {
        return Err(format!("Content-Type {ct:?}, expected text/javascript"));
    }
    let h = crate::sha256_hex(&r.body);
    match EMBED_RELEASES.iter().position(|e| e.sha256 == h) {
        Some(i) if i + 1 == EMBED_RELEASES.len() => {
            Ok(format!("fastkiwi-embed {}", EMBED_RELEASES[i].version))
        }
        Some(i) => Ok(format!(
            "fastkiwi-embed {} (older; current is {})",
            EMBED_RELEASES[i].version,
            EMBED_RELEASES[EMBED_RELEASES.len() - 1].version
        )),
        None => Err(format!("sha256 {h} is not a released embed.js")),
    }
}

/// The opening tags `<name …>` in `html` (attribute text only).
fn tags(html: &str, name: &str) -> Vec<String> {
    let lower = html.to_ascii_lowercase();
    let open = format!("<{name}");
    let mut out = Vec::new();
    let mut at = 0;
    while let Some(i) = lower[at..].find(&open) {
        let start = at + i + open.len();
        let next = lower.as_bytes().get(start).copied().unwrap_or(b'>');
        if !(next == b' '
            || next == b'\t'
            || next == b'\n'
            || next == b'\r'
            || next == b'>'
            || next == b'/')
        {
            at = start;
            continue;
        }
        let end = lower[start..].find('>').map_or(lower.len(), |e| start + e);
        out.push(html[start..end].to_string());
        at = end;
    }
    out
}

/// The value of attribute `name` in a tag's attribute text (double or single quoted).
fn attr(tag: &str, name: &str) -> Option<String> {
    let lower = tag.to_ascii_lowercase();
    let mut at = 0;
    while let Some(i) = lower[at..].find(name) {
        let s = at + i;
        let before_ok = s == 0 || lower.as_bytes()[s - 1].is_ascii_whitespace();
        let rest = lower[s + name.len()..].trim_start();
        if before_ok && rest.starts_with('=') {
            let off = lower.len() - rest.len() + 1;
            let v = tag[off..].trim_start();
            let q = v.chars().next()?;
            if q == '"' || q == '\'' {
                return v[1..].find(q).map(|e| v[1..1 + e].to_string());
            }
            return Some(v.split_ascii_whitespace().next().unwrap_or("").to_string());
        }
        at = s + name.len();
    }
    None
}

fn hub_update(exe: &Path, opts: &Options) -> Outcome {
    let mut r = match Running::spawn(exe, &["--hub-update", "--check-only", "--hub-owner", "0"]) {
        Ok(r) => r,
        Err(e) => return fail(format!("cannot start: {e}")),
    };
    let mut run = UpdateRun::new(true);
    let mut wait = UPDATE_FIRST_LINE;
    loop {
        match r.next_line(wait) {
            Line::Text(l) => {
                let Ok(text) = String::from_utf8(l) else {
                    return fail("a line is not UTF-8");
                };
                let parsed = match hubcli::parse_update_line(&text) {
                    Ok(p) => p,
                    Err(e) => return fail(format!("line {}: {e}", run.lines() + 1)),
                };
                if let Err(e) = run.push(&parsed) {
                    return fail(format!("line {}: {e}", run.lines() + 1));
                }
                wait = opts.update_silence;
            }
            Line::Eof => break,
            Line::Timeout => {
                r.kill();
                return fail(format!(
                    "silent for {} s (line {})",
                    wait.as_secs_f32(),
                    run.lines() + 1
                ));
            }
        }
    }
    let code = r
        .wait_timeout(Duration::from_secs(5))
        .and_then(|s| s.code());
    match run.finish(code) {
        Err(e) => fail(e.to_string()),
        Ok(last) => pass(match (last.state, &last.version, last.code) {
            (UpdateState::Available, Some(v), _) => format!("available {v}"),
            (UpdateState::Error, _, Some(c)) => format!("error {}", kebab(&c)),
            (s, _, _) => kebab(&s),
        }),
    }
}

fn kebab<T: serde::Serialize>(v: &T) -> String {
    serde_json::to_value(v)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_default()
}

fn temp_base() -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    std::env::temp_dir().join(format!("fk-check-{}-{nanos}", std::process::id()))
}

/// Run `--export-data <dir>`; returns (exit code, stdout lines).
fn run_export(exe: &Path, dir: &Path) -> Result<(Option<i32>, Vec<String>), String> {
    let dir = dir.to_str().ok_or("temp path is not UTF-8")?;
    let mut r =
        Running::spawn(exe, &["--export-data", dir]).map_err(|e| format!("cannot start: {e}"))?;
    r.close_stdin();
    let mut lines = Vec::new();
    let end = Instant::now() + Duration::from_secs(120);
    loop {
        match r.next_line(end.saturating_duration_since(Instant::now())) {
            Line::Text(l) => lines.push(String::from_utf8_lossy(&l).into_owned()),
            Line::Eof => break,
            Line::Timeout => {
                r.kill();
                return Err("did not finish within 120 s".into());
            }
        }
    }
    let code = r
        .wait_timeout(Duration::from_secs(5))
        .and_then(|s| s.code());
    Ok((code, lines))
}

fn list_files(root: &Path) -> std::io::Result<Vec<String>> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d)? {
            let e = e?;
            let t = e.file_type()?;
            let rel = e
                .path()
                .strip_prefix(root)
                .expect("under root")
                .to_string_lossy()
                .replace('\\', "/");
            if t.is_symlink() {
                out.push(format!("{rel} (link)"));
            } else if t.is_dir() {
                stack.push(e.path());
            } else {
                out.push(rel);
            }
        }
    }
    out.sort();
    Ok(out)
}

fn export(exe: &Path, app: &str, rep: &mut Report) {
    let base = temp_base();
    if let Err(e) = std::fs::create_dir_all(&base) {
        rep.add("E1", fail(format!("temp folder: {e}")));
        rep.add("E2", Outcome::Skip("needs a temp folder".into()));
        return;
    }
    let out = base.join("export");
    let o = (|| -> Result<String, String> {
        let (code, lines) = run_export(exe, &out)?;
        let [line] = lines.as_slice() else {
            return Err(format!("{} stdout lines, expected 1", lines.len()));
        };
        let res = hubcli::parse_export_result(line).map_err(|e| e.to_string())?;
        if res.state != ExportState::Done {
            return Err(format!("state error, code {}", kebab(&res.code)));
        }
        if code != Some(exit::OK) {
            return Err(format!("exit code {code:?}, expected 0"));
        }
        let text = std::fs::read_to_string(out.join(EXPORT_MANIFEST))
            .map_err(|e| format!("{EXPORT_MANIFEST}: {e}"))?;
        let m = hubcli::parse_export_manifest(&text, app).map_err(|e| e.to_string())?;
        let mut expected: Vec<String> = m.files.iter().map(|f| f.path.clone()).collect();
        expected.push(EXPORT_MANIFEST.to_string());
        expected.sort();
        let found = list_files(&out).map_err(|e| e.to_string())?;
        if found != expected {
            let extra: Vec<_> = found.iter().filter(|f| !expected.contains(f)).collect();
            let missing: Vec<_> = expected.iter().filter(|f| !found.contains(f)).collect();
            return Err(format!(
                "files differ from the manifest: extra {extra:?}, missing {missing:?}"
            ));
        }
        for e in std::fs::read_dir(&out)
            .map_err(|e| e.to_string())?
            .flatten()
        {
            let n = e.file_name().to_string_lossy().into_owned();
            if n != EXPORT_MANIFEST && n != EXPORT_DATA_DIR {
                return Err(format!("unexpected top-level entry {n:?}"));
            }
        }
        let mut bytes = 0u64;
        for f in &m.files {
            let b = std::fs::read(out.join(&f.path)).map_err(|e| format!("{}: {e}", f.path))?;
            if b.len() as u64 != f.size {
                return Err(format!(
                    "{}: size {} in the manifest, {} on disk",
                    f.path,
                    f.size,
                    b.len()
                ));
            }
            if crate::sha256_hex(&b) != f.sha256 {
                return Err(format!("{}: sha256 does not match", f.path));
            }
            bytes += f.size;
        }
        if res.files != Some(m.files.len() as u64) || res.bytes != Some(bytes) {
            return Err("the result line's files/bytes do not match the manifest".into());
        }
        Ok(format!("{} file(s), {bytes} bytes", m.files.len()))
    })();
    rep.add("E1", o.map_or_else(fail, pass));

    let full = base.join("full");
    let o = (|| -> Result<(), String> {
        std::fs::create_dir_all(&full).map_err(|e| e.to_string())?;
        std::fs::write(full.join("keep.txt"), b"keep").map_err(|e| e.to_string())?;
        let (code, lines) = run_export(exe, &full)?;
        let [line] = lines.as_slice() else {
            return Err(format!("{} stdout lines, expected 1", lines.len()));
        };
        let res = hubcli::parse_export_result(line).map_err(|e| e.to_string())?;
        if res.state != ExportState::Error || res.code != Some(ExportCode::NotEmpty) {
            return Err("expected state error, code not-empty".into());
        }
        if code != Some(exit::FAILED) {
            return Err(format!("exit code {code:?}, expected 1"));
        }
        let found = list_files(&full).map_err(|e| e.to_string())?;
        if found != ["keep.txt"]
            || std::fs::read(full.join("keep.txt")).ok().as_deref() != Some(b"keep")
        {
            return Err("the folder was changed".into());
        }
        Ok(())
    })();
    rep.add("E2", o.map_or_else(fail, |_| pass("")));
    let _ = std::fs::remove_dir_all(&base);
}

fn usage(exe: &Path, commands: &[Command]) -> Outcome {
    let mut notes = Vec::new();
    for c in commands {
        let args: Vec<&str> = match c {
            Command::HubHost => vec!["--hub-host", "--fastkiwi-no-such-flag"],
            Command::HubUpdate => vec!["--hub-update", "--fastkiwi-no-such-flag"],
            Command::ExportData => vec!["--export-data"],
        };
        let mut r = match Running::spawn(exe, &args) {
            Ok(r) => r,
            Err(e) => return fail(format!("cannot start: {e}")),
        };
        r.close_stdin();
        let code = r
            .wait_timeout(Duration::from_secs(5))
            .and_then(|s| s.code());
        let mut out = r.pending_lines();
        while let Line::Text(l) = r.next_line(Duration::from_millis(200)) {
            out.push(l);
        }
        if code != Some(exit::USAGE) {
            r.kill();
            return fail(format!(
                "{}: exit code {code:?}, expected 2",
                args.join(" ")
            ));
        }
        if !out.is_empty() {
            return fail(format!("{}: printed {} line(s)", args.join(" "), out.len()));
        }
        notes.push(args.join(" "));
    }
    pass(notes.join("; "))
}

#[cfg(windows)]
fn visible_windows(pid: u32) -> usize {
    use windows::Win32::Foundation::{HWND, LPARAM};
    use windows::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetWindowThreadProcessId, IsWindowVisible,
    };
    use windows::core::BOOL;
    struct Ctx {
        pid: u32,
        n: usize,
    }
    unsafe extern "system" fn cb(h: HWND, l: LPARAM) -> BOOL {
        // SAFETY: `l` is the address of the `Ctx` below, alive for the whole EnumWindows call.
        let ctx = unsafe { &mut *(l.0 as *mut Ctx) };
        let mut p = 0u32;
        // SAFETY: plain Win32 queries on a window handle EnumWindows gave us.
        unsafe {
            GetWindowThreadProcessId(h, Some(&mut p));
            if p == ctx.pid && IsWindowVisible(h).as_bool() {
                ctx.n += 1;
            }
        }
        BOOL(1)
    }
    let mut ctx = Ctx { pid, n: 0 };
    // SAFETY: the callback only touches `ctx` through the pointer passed here.
    unsafe {
        let _ = EnumWindows(Some(cb), LPARAM(&mut ctx as *mut Ctx as isize));
    }
    ctx.n
}

#[cfg(not(windows))]
fn visible_windows(_pid: u32) -> usize {
    0
}

/// A request the Origin guard must refuse: (method, what, extra headers).
type Refused<'a> = (&'a str, &'a str, Vec<(&'a str, &'a str)>);
