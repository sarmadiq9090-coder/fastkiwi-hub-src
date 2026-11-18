//! FastKiwi-Hub.exe: the Arabic smart bar and the Hub window.
//!
//! One tao window for the Hub and one small window for the bar (opened by a global shortcut
//! registered with `RegisterHotKey`, never a keyboard hook). Both are WebView2 pages served
//! over the `fastkiwi` custom protocol (no local HTTP server). A hosted module gets its own
//! child WebView2 with its own user-data folder, a navigation allowlist, no host objects, and
//! only `fastkiwi-module@1` web messages.
//!
//! Detection is read only. Launches are ShellExecute and forget, of an exe detection found
//! (or a program the person picked), never with arguments from the catalog. Online: the
//! signed catalog from fastkiwi.app, GitHub release metadata, and a local Ollama on
//! 127.0.0.1 (`net.rs`, the only network code). The Hub never downloads or installs a
//! program: installers open in the browser.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[cfg(not(windows))]
fn main() {
    eprintln!("FastKiwi Hub runs on Windows only.");
}

#[cfg(windows)]
fn main() {
    hub::run();
}

#[cfg(windows)]
mod hub {
    use kiwi_hub::allowlist;
    use kiwi_hub::catalog::{self, App as CatApp, Kind, Platform};
    use kiwi_hub::findcli;
    use kiwi_hub::hotkey::{self, windows::Hotkey};
    use kiwi_hub::links;
    use kiwi_hub::mine::{self, Program};
    use kiwi_hub::online;
    use kiwi_hub::updates::{self, Check};
    use kiwi_hub::detect::{self, AppDetection};
    use kiwi_hub::hosting::{self, UpdateRoute};
    use kiwi_hub::hostproc::{self, HostEvent, HostProcess, UpdateEvent, UpdateHandle};
    use kiwi_hub::protocol::{self, Digits, HubMsg, Lang, ModuleMsg, Theme};
    use kiwi_hub::serve;
    use kiwi_hub::settings::{self, Settings};
    use kiwi_hub::win::{self, Target};
    use serde_json::{Value, json};
    use std::borrow::Cow;
    use std::collections::HashMap;
    use std::path::PathBuf;
    use std::time::{Duration, Instant};
    use tao::dpi::{LogicalPosition, LogicalSize};
    use tao::event::{Event, WindowEvent};
    use tao::event_loop::{ControlFlow, EventLoopBuilder, EventLoopProxy};
    use tao::platform::windows::{IconExtWindows, WindowExtWindows};
    use tao::window::{Icon, WindowBuilder};
    use wry::http::{Request, Response, StatusCode, header};
    use wry::{
        NewWindowResponse, PageLoadEvent, PermissionResponse, Rect, WebContext, WebView,
        WebViewBuilder, WebViewBuilderExtWindows, WebViewExtWindows,
    };

    const VERSION: &str = env!("CARGO_PKG_VERSION");
    /// Integration runs: what the hosted page shows after the Hub's settings arrived.
    const PROBE_JS: &str = r#"(() => { try {
  const h = document.documentElement;
  const brand = [...document.querySelectorAll("[data-fk-brand]")];
  return JSON.stringify({
    url: location.origin + location.pathname, title: document.title,
    marker: !!window.__FASTKIWI_HUB__, hosted: h.hasAttribute("data-fk-hosted"),
    theme: h.dataset.theme ?? null, tone: h.dataset.tone ?? null, lang: h.lang, dir: h.dir,
    digits: h.dataset.digits ?? null,
    brand: brand.length, brand_hidden: brand.filter((e) => getComputedStyle(e).display === "none").length,
    sample: (document.body?.innerText ?? "").slice(0, 160)
  });
} catch (e) { return JSON.stringify({ error: String(e) }); } })()"#;
    /// Integration runs: the page asks for its own window through its own copy of embed.js.
    const OPEN_OWN_JS: &str =
        r#"import("/__fastkiwi/embed.js").then((m) => m.openOwnWindow()).catch(() => {});"#;
    /// A module that has not said `ready` this long after its page loaded does not speak the
    /// protocol (yet): the Hub says so in the module bar.
    const READY_TIMEOUT: Duration = Duration::from_millis(2500);
    /// Ignore repeated launch requests for the same app within this time.
    const LAUNCH_GAP: Duration = Duration::from_secs(2);

    #[derive(Debug)]
    enum Ev {
        HubIpc(String),
        ModuleIpc {
            app: String,
            body: String,
        },
        ModuleLoaded {
            app: String,
            epoch: u64,
        },
        ModuleTimeout {
            app: String,
            epoch: u64,
        },
        Blocked {
            who: String,
            host: String,
        },
        Detected(Vec<AppDetection>),
        Host {
            app: String,
            epoch: u64,
            ev: HostEvent,
        },
        Update {
            app: String,
            run: u64,
            ev: UpdateEvent,
        },
        ShotNow,
        ShotTimeout,
        /// Integration runs: read the hosted page's state, then ask it to open its own window.
        Probe {
            app: String,
        },
        ProbeResult {
            app: String,
            json: String,
        },
        /// A message from the bar window's page.
        BarIpc(String),
        /// The global shortcut was pressed.
        Hotkey,
        HotkeyStatus(hotkey::Status),
        /// A newer signed catalog arrived from fastkiwi.app.
        Catalog(Box<catalog::Catalog>),
        /// Update checks finished: (id, result) per app, plus InstaFind's own version.
        Checks {
            rows: Vec<(String, Value)>,
            instafind: Option<String>,
        },
        IconsFetched,
        FindDone {
            seq: u64,
            hits: Result<Vec<findcli::Hit>, String>,
        },
        Ollama(Option<String>),
        AskDone {
            seq: u64,
            answer: Result<String, String>,
        },
    }

    /// Which of the Hub's own pages sent a message.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Page {
        Hub,
        Bar,
    }

    #[derive(Debug, Clone)]
    struct Args {
        dev: bool,
        dev_module_dir: Option<PathBuf>,
        catalog: Option<PathBuf>,
        detect_json: Option<PathBuf>,
        shot: Option<Shot>,
        /// `--dev-host <id>=<exe>`: treat that exe (and the `fastkiwi-hub.json` next to it) as a
        /// portable copy of the app, e.g. `stub-app.exe` from `tools/conformance`.
        dev_hosts: Vec<(String, PathBuf)>,
        /// `--dev-report <file.json>` (with `--dev-shot`): what happened, for integration runs.
        dev_report: Option<PathBuf>,
    }

    #[derive(Debug, Clone)]
    struct Shot {
        out: PathBuf,
        screen: String,
        theme: String,
        lang: String,
    }

    fn parse_args() -> Args {
        let mut a = Args {
            dev: false,
            dev_module_dir: None,
            catalog: None,
            detect_json: None,
            shot: None,
            dev_hosts: Vec::new(),
            dev_report: None,
        };
        let mut it = std::env::args().skip(1);
        let (mut screen, mut theme, mut lang, mut out) = (
            "home".to_string(),
            "light".to_string(),
            "ar".to_string(),
            None,
        );
        while let Some(x) = it.next() {
            match x.as_str() {
                "--dev" => a.dev = true,
                "--dev-module-dir" => {
                    a.dev = true;
                    a.dev_module_dir = it.next().map(PathBuf::from);
                }
                "--catalog" => {
                    a.dev = true;
                    a.catalog = it.next().map(PathBuf::from);
                }
                "--detect-json" => a.detect_json = it.next().map(PathBuf::from),
                "--dev-report" => a.dev_report = it.next().map(PathBuf::from),
                "--dev-host" => {
                    a.dev = true;
                    if let Some((id, exe)) = it.next().as_deref().and_then(|v| v.split_once('='))
                        && catalog::valid_id(id)
                    {
                        a.dev_hosts.push((id.to_string(), PathBuf::from(exe)));
                    }
                }
                "--dev-shot" => out = it.next().map(PathBuf::from),
                "--screen" => screen = it.next().unwrap_or(screen),
                "--theme" => theme = it.next().unwrap_or(theme),
                "--lang" => lang = it.next().unwrap_or(lang),
                _ => {}
            }
        }
        a.shot = out.map(|out| Shot {
            out,
            screen,
            theme,
            lang,
        });
        a
    }

    /// Where a hosted module's files come from.
    #[derive(Debug, Clone)]
    enum ModuleSource {
        /// The installed app's `web/` folder.
        Installed { root: PathBuf, entry: String },
        /// `--dev-module-dir` (e.g. a local web build of the app).
        DevDir { root: PathBuf, entry: String },
        /// The small stub module compiled into the Hub (dev only).
        Stub,
        /// A kind P app's own `--hub-host` page.
        Process { port: u16 },
    }

    impl ModuleSource {
        fn label(&self) -> &'static str {
            match self {
                ModuleSource::Installed { .. } => "installed",
                ModuleSource::DevDir { .. } => "dev-dir",
                ModuleSource::Stub => "stub",
                ModuleSource::Process { .. } => "process",
            }
        }
        fn entry(&self) -> String {
            match self {
                ModuleSource::Installed { entry, .. } | ModuleSource::DevDir { entry, .. } => {
                    entry.clone()
                }
                ModuleSource::Stub | ModuleSource::Process { .. } => "index.html".into(),
            }
        }
    }

    struct ModuleView {
        webview: WebView,
        _ctx: WebContext,
        epoch: u64,
        ready: bool,
        source: ModuleSource,
    }

    struct Hub {
        args: Args,
        window: tao::window::Window,
        hub: WebView,
        apps: Vec<CatApp>,
        detection: Option<Vec<AppDetection>>,
        settings: Settings,
        resolved: (Theme, Lang, Digits),
        modules: HashMap<String, ModuleView>,
        epoch: u64,
        bounds: (f64, f64, f64, f64),
        module_visible: bool,
        /// The app whose tab is in front (a module view built later must not cover another).
        shown_app: String,
        last_launch: HashMap<String, Instant>,
        proxy: EventLoopProxy<Ev>,
        /// `--hub-host` processes, by app id, with the epoch of the tab that started them.
        hosts: HashMap<String, (u64, HostProcess)>,
        /// Running `--hub-update` runs, by app id.
        runs: HashMap<String, Run>,
        run_seq: u64,
        /// The last update state per app (the Updates screen).
        upd: HashMap<String, Value>,
        /// Each app's update check runs at most once per Hub start by itself (setting on).
        auto_checked: bool,
        shot_tries: u32,
        /// `--dev-report` events, written when a shot run ends.
        report: Vec<Value>,
        started: Instant,
        /// The Hub's own folder; `None` in screenshot runs, which write nothing.
        dir: Option<PathBuf>,
        catalog_serial: u64,
        catalog_from_cache: bool,
        /// Programs the person added.
        mine: Vec<Program>,
        /// Update check results by app id (`fastkiwi-hub` for the Hub itself).
        checks: HashMap<String, Value>,
        checking: bool,
        /// InstaFind's version, from its own `--version` (its exe has no version resource).
        instafind_version: Option<String>,
        /// The bar window and its page (not in screenshot runs).
        bar: Option<(tao::window::Window, WebView)>,
        hotkey: Option<Hotkey>,
        hotkey_status: Value,
        last_hits: Vec<findcli::Hit>,
        ollama: Option<String>,
        ollama_probed: Option<Instant>,
        history: Vec<String>,
        _ctx: WebContext,
    }

    struct Run {
        id: u64,
        check_only: bool,
        handle: UpdateHandle,
    }

    fn respond(
        status: StatusCode,
        mime: &str,
        csp: &str,
        body: Cow<'static, [u8]>,
    ) -> Response<Cow<'static, [u8]>> {
        Response::builder()
            .status(status)
            .header(header::CONTENT_TYPE, mime)
            .header("Content-Security-Policy", csp)
            .header("X-Content-Type-Options", "nosniff")
            .header("Referrer-Policy", "no-referrer")
            .header(header::CACHE_CONTROL, "no-store")
            .body(body)
            .unwrap_or_else(|_| Response::new(Cow::Borrowed(&[][..])))
    }

    fn not_found(csp: &str) -> Response<Cow<'static, [u8]>> {
        respond(
            StatusCode::NOT_FOUND,
            "text/plain; charset=utf-8",
            csp,
            Cow::Borrowed(b"not found"),
        )
    }

    fn serve_hub(req: Request<Vec<u8>>) -> Response<Cow<'static, [u8]>> {
        if req.uri().host() != Some("hub") || req.method() != wry::http::Method::GET {
            return not_found(serve::HUB_CSP);
        }
        // Icons the Hub keeps in its own folder (catalog icons from fastkiwi.app, icons of
        // programs the person added). Shown as images only.
        if let Some(name) = req.uri().path().strip_prefix("/local-icons/") {
            return match (serve::local_icon_name_ok(name), settings::hub_dir()) {
                (true, Some(d)) => match std::fs::read(d.join("icons").join(name)) {
                    Ok(b) if b.len() <= 512 * 1024 => respond(
                        StatusCode::OK,
                        serve::mime(name),
                        serve::HUB_CSP,
                        Cow::Owned(b),
                    ),
                    _ => not_found(serve::HUB_CSP),
                },
                _ => not_found(serve::HUB_CSP),
            };
        }
        match serve::clean_rel_path(req.uri().path(), "index.html")
            .and_then(|rel| serve::ui_file(&rel).map(|b| (rel, b)))
        {
            Some((rel, bytes)) => respond(
                StatusCode::OK,
                serve::mime(&rel),
                serve::HUB_CSP,
                Cow::Borrowed(bytes),
            ),
            None => not_found(serve::HUB_CSP),
        }
    }

    fn serve_module(
        id: &str,
        src: &ModuleSource,
        req: Request<Vec<u8>>,
    ) -> Response<Cow<'static, [u8]>> {
        let csp = serve::MODULE_CSP;
        if req.uri().host() != Some(id) || req.method() != wry::http::Method::GET {
            return not_found(csp);
        }
        let Some(rel) = serve::clean_rel_path(req.uri().path(), &src.entry()) else {
            return not_found(csp);
        };
        let body: Option<Cow<'static, [u8]>> = match src {
            ModuleSource::Installed { root, .. } | ModuleSource::DevDir { root, .. } => {
                serve::resolve_in_root(root, &rel)
                    .and_then(|p| std::fs::read(p).ok())
                    .map(Cow::Owned)
            }
            ModuleSource::Stub => serve::stub_file(&rel).map(Cow::Borrowed),
            ModuleSource::Process { .. } => None,
        };
        match body {
            Some(b) => respond(StatusCode::OK, serve::mime(&rel), csp, b),
            None => not_found(csp),
        }
    }

    /// Turn off what a hosted page never needs. Host objects stay off for every WebView.
    fn harden(wv: &WebView, dev: bool, is_hub: bool) {
        // SAFETY: COM calls on the WebView's own settings object, on the UI thread.
        unsafe {
            if let Ok(s) = wv.webview().Settings() {
                let _ = s.SetAreHostObjectsAllowed(false);
                let _ = s.SetIsStatusBarEnabled(false);
                let _ = s.SetAreDevToolsEnabled(dev);
                // No autofill or saved passwords in any page the Hub shows.
                if let Ok(s4) = windows::core::Interface::cast::<
                    webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2Settings4,
                >(&s)
                {
                    let _ = s4.SetIsGeneralAutofillEnabled(false);
                    let _ = s4.SetIsPasswordAutosaveEnabled(false);
                }
                if is_hub {
                    let _ = s.SetIsZoomControlEnabled(false);
                }
            }
        }
    }

    /// Receive a WebView's web messages (`chrome.webview.postMessage` with a string) without
    /// wry's IPC handler, so the page gets no extra global (`window.ipc`). `f(source, body)`.
    fn on_web_message(wv: &WebView, f: impl Fn(String, String) + 'static) {
        use webview2_com::{WebMessageReceivedEventHandler, take_pwstr};
        use windows::core::PWSTR;
        let handler = WebMessageReceivedEventHandler::create(Box::new(move |_, args| {
            let Some(args) = args else {
                return Ok(());
            };
            // SAFETY: COM out-parameters filled by WebView2 and freed by take_pwstr.
            unsafe {
                let mut src = PWSTR::null();
                args.Source(&mut src)?;
                let source = take_pwstr(src);
                let mut text = PWSTR::null();
                // Non-string messages (postMessage of an object) are not part of the contract.
                if args.TryGetWebMessageAsString(&mut text).is_ok() {
                    f(source, take_pwstr(text));
                }
            }
            Ok(())
        }));
        let mut token = 0i64;
        // SAFETY: COM call on the UI thread; the handler lives as long as the WebView.
        unsafe {
            let _ = wv.webview().add_WebMessageReceived(&handler, &mut token);
        }
    }

    fn post(wv: &WebView, msg: &HubMsg) {
        let text = protocol::encode_hub_msg(msg);
        // SAFETY: COM call on the UI thread with a NUL-terminated string.
        unsafe {
            let _ = wv
                .webview()
                .PostWebMessageAsString(&windows::core::HSTRING::from(text));
        }
    }

    /// Screenshot runs use their own WebView2 profiles (other browser arguments).
    static SHOT_MODE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

    fn data_dir(sub: &str) -> PathBuf {
        let shot = SHOT_MODE.load(std::sync::atomic::Ordering::Relaxed);
        settings::hub_dir()
            .unwrap_or_else(std::env::temp_dir)
            .join(if shot { "WebView2-shot" } else { "WebView2" })
            .join(sub)
    }

    /// wry's default WebView2 arguments, plus (screenshot runs only) keep painting while the
    /// window is off screen.
    fn browser_args() -> String {
        let mut a = String::from("--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection");
        if SHOT_MODE.load(std::sync::atomic::Ordering::Relaxed) {
            a.push_str(",CalculateNativeWinOcclusion --disable-backgrounding-occluded-windows --disable-renderer-backgrounding");
        }
        a
    }

    /// The catalog to start with: `--catalog <file>` (dev), else the signed copy cached from
    /// fastkiwi.app when it is still good, else the copy inside the exe.
    fn load_catalog(args: &Args, dir: Option<&std::path::Path>) -> (Vec<CatApp>, u64, bool) {
        if let Some(text) = args
            .catalog
            .as_ref()
            .and_then(|p| std::fs::read_to_string(p).ok())
        {
            return match catalog::parse(&text) {
                Ok(c) => (catalog::visible(&c, args.dev), c.serial, false),
                Err(e) => {
                    eprintln!("catalog: {e}");
                    (Vec::new(), 0, false)
                }
            };
        }
        let cached = online::cached_catalog(dir);
        let (c, from_cache) = catalog::choose(
            catalog::FIXTURE,
            cached.as_ref().map(|(t, s)| (t.as_str(), s.as_str())),
            &online::today(),
            online::seen_serial(dir),
        );
        (catalog::visible(&c, args.dev), c.serial, from_cache)
    }

    fn spawn_detection(apps: Vec<CatApp>, proxy: EventLoopProxy<Ev>) {
        std::thread::spawn(move || {
            // SAFETY: COM/WinRT for this worker thread (Store package queries).
            unsafe {
                let _ = windows::Win32::System::Com::CoInitializeEx(
                    None,
                    windows::Win32::System::Com::COINIT_MULTITHREADED,
                );
            }
            let d = detect::detect_all(&detect::windows::WindowsView, &apps);
            let _ = proxy.send_event(Ev::Detected(d));
        });
    }

    fn write_detect_json(args: &Args, out: &PathBuf) {
        // SAFETY: COM/WinRT for this thread.
        unsafe {
            let _ = windows::Win32::System::Com::CoInitializeEx(
                None,
                windows::Win32::System::Com::COINIT_MULTITHREADED,
            );
        }
        let apps = load_catalog(args, settings::hub_dir().as_deref()).0;
        let d = detect::detect_all(&detect::windows::WindowsView, &apps);
        let text =
            serde_json::to_string_pretty(&json!({ "hub": VERSION, "apps": d })).unwrap_or_default();
        let _ = std::fs::write(out, text);
    }

    impl Hub {
        fn send_ui(&self, v: Value) {
            let _ = self
                .hub
                .evaluate_script(&format!("window.__hub && window.__hub.recv({v})"));
        }

        fn send_bar(&self, v: Value) {
            if let Some((_, wv)) = &self.bar {
                let _ = wv.evaluate_script(&format!("window.__hub && window.__hub.recv({v})"));
            }
        }

        fn send_to(&self, page: Page, v: Value) {
            match page {
                Page::Hub => self.send_ui(v),
                Page::Bar => self.send_bar(v),
            }
        }

        fn send_all(&self, v: Value) {
            self.send_bar(v.clone());
            self.send_ui(v);
        }

        fn app(&self, id: &str) -> Option<&CatApp> {
            self.apps.iter().find(|a| a.id == id)
        }

        fn installs(&self, id: &str) -> &[detect::Install] {
            self.detection
                .as_ref()
                .and_then(|d| d.iter().find(|x| x.id == id))
                .map(|x| x.installs.as_slice())
                .unwrap_or(&[])
        }

        /// Where the module for `id` would come from, if it can be hosted now.
        fn module_source(&self, id: &str) -> Option<ModuleSource> {
            let app = self.app(id)?;
            let hub = app.hub.as_ref()?;
            if hub.kind != Kind::S {
                return None;
            }
            let m = hub.module.as_ref()?;
            if app.third_party() {
                return None; // no third-party modules in 0.3.0
            }
            // Only a copy that declares hub mode in its fastkiwi-hub.json (council C4).
            if let Some(entry) = self
                .installs(id)
                .iter()
                .filter(|i| i.hub.is_some())
                .find_map(|i| i.web_entry.clone())
            {
                let root = PathBuf::from(entry).parent()?.to_path_buf();
                return Some(ModuleSource::Installed {
                    root,
                    entry: m.entry.clone(),
                });
            }
            if !self.args.dev {
                return None;
            }
            if id == "instawords"
                && let Some(dir) = &self.args.dev_module_dir
                && dir.join(&m.entry).is_file()
            {
                return Some(ModuleSource::DevDir {
                    root: dir.clone(),
                    entry: m.entry.clone(),
                });
            }
            Some(ModuleSource::Stub)
        }

        fn detection_json(&self) -> Value {
            match &self.detection {
                None => Value::Null,
                Some(d) => Value::Array(
                    d.iter()
                        .map(|x| {
                            let mut v = serde_json::to_value(x).unwrap_or(Value::Null);
                            if let Value::Object(o) = &mut v {
                                let label = self
                                    .module_source(&x.id)
                                    .map(|s| s.label())
                                    .or_else(|| self.host_install(&x.id).map(|_| "process"));
                                o.insert(
                                    "module".into(),
                                    label
                                        .map(|l| Value::String(l.into()))
                                        .unwrap_or(Value::Null),
                                );
                                o.insert(
                                    "update".into(),
                                    serde_json::to_value(hosting::update_route(&x.installs))
                                        .unwrap_or(Value::Null),
                                );
                            }
                            v
                        })
                        .collect(),
                ),
            }
        }

        fn state(&self) -> Value {
            let apps: Vec<Value> = self.apps.iter().map(|a| self.app_json(a)).collect();
            json!({
                "type": "state",
                "version": VERSION,
                "dev": self.args.dev,
                "apps": apps,
                "detection": self.detection_json(),
                "settings": self.settings,
                "upd": self.upd,
                "checks": self.checks,
                "checking": self.checking,
                "mine": self.mine_json(),
                "catalog": {"serial": self.catalog_serial, "from_cache": self.catalog_from_cache},
                "hotkey": self.hotkey_status,
                "find": self.find_exe().is_some(),
                "ollama": self.ollama.is_some(),
                "history": if self.settings.bar_history { json!(self.history) } else { json!([]) },
                "shot": self.args.shot.as_ref().map(|s| json!({"screen": s.screen, "theme": s.theme, "lang": s.lang})),
            })
        }

        fn launch(&mut self, id: &str, index: usize) -> Result<(), String> {
            let now = Instant::now();
            if self
                .last_launch
                .get(id)
                .is_some_and(|t| now.duration_since(*t) < LAUNCH_GAP)
            {
                return Err("busy".into());
            }
            let inst = self
                .installs(id)
                .get(index)
                .cloned()
                .ok_or("no such install")?;
            if !inst.launchable {
                return Err("missing exe".into());
            }
            let target = match (&inst.aumid, &inst.exe) {
                (Some(a), _) => Target::Aumid(a.clone()),
                (None, Some(e)) => Target::Exe(e.clone()),
                _ => return Err("nothing to launch".into()),
            };
            self.last_launch.insert(id.to_string(), now);
            if self.args.shot.is_some() {
                // Off-screen runs never open a window on the PC (D-H11): record what would start.
                let kind = match &target {
                    Target::Exe(e) => format!(
                        "exe {}",
                        std::path::Path::new(e)
                            .file_name()
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_default()
                    ),
                    Target::Aumid(a) => format!("aumid {a}"),
                    Target::Picked(_) => "picked".into(),
                };
                self.rec(json!({"ev": "launch", "app": id, "target": kind, "dry_run": true}));
                return Ok(());
            }
            win::launch(&target)
        }

        fn first_launchable(&self, id: &str) -> Option<usize> {
            self.installs(id).iter().position(|i| i.launchable)
        }

        /// The copy of a kind P app to run with `--hub-host`, if any declares it.
        fn host_install(&self, id: &str) -> Option<usize> {
            let app = self.app(id)?;
            if app.third_party() {
                return None;
            }
            hosting::host_install(app, self.installs(id))
        }

        fn open_module(&mut self, id: &str) {
            self.shown_app = id.to_string();
            if let Some(m) = self.modules.get(id) {
                let _ = m.webview.set_visible(true);
                self.module_visible = true;
                self.send_ui(json!({"type": "module", "app": id, "status": if m.ready { "ready" } else { "loading" }, "source": m.source.label()}));
                return;
            }
            if self.hosts.contains_key(id) {
                self.send_ui(
                    json!({"type": "module", "app": id, "status": "starting", "source": "process"}),
                );
                return;
            }
            if let Some(source) = self.module_source(id) {
                self.epoch += 1;
                let url = allowlist::module_start_url(id, &source.entry());
                self.module_visible = true;
                self.build_module_view(id, source, url, self.epoch);
                return;
            }
            let Some(idx) = self.host_install(id) else {
                self.send_ui(json!({"type": "module", "app": id, "status": "unavailable"}));
                return;
            };
            let Some(exe) = self.installs(id)[idx].exe.clone() else {
                return;
            };
            self.epoch += 1;
            let epoch = self.epoch;
            let p = self.proxy.clone();
            let app = id.to_string();
            match HostProcess::start(
                std::path::Path::new(&exe),
                id,
                hubcli::HANDSHAKE_TIMEOUT,
                move |ev| {
                    let _ = p.send_event(Ev::Host {
                        app: app.clone(),
                        epoch,
                        ev,
                    });
                },
            ) {
                Ok(proc) => {
                    self.hosts.insert(id.to_string(), (epoch, proc));
                    self.module_visible = true;
                    self.send_ui(json!({"type": "module", "app": id, "status": "starting", "source": "process"}));
                }
                Err(e) => {
                    self.trace(&format!("{id}: --hub-host did not start: {e}"));
                    self.send_ui(json!({"type": "module", "app": id, "status": "host-failed"}));
                }
            }
        }

        fn rec(&mut self, mut v: Value) {
            if self.args.dev_report.is_none() {
                return;
            }
            if let Value::Object(o) = &mut v {
                o.insert(
                    "ms".into(),
                    json!(self.started.elapsed().as_millis() as u64),
                );
            }
            self.report.push(v);
        }

        fn write_report(&self) {
            if let Some(out) = &self.args.dev_report {
                let s = self.args.shot.as_ref();
                let v = json!({
                    "hub": VERSION,
                    "screen": s.map(|s| &s.screen), "theme": s.map(|s| &s.theme), "lang": s.map(|s| &s.lang),
                    "resolved": {"theme": self.resolved.0, "lang": self.resolved.1, "digits": self.resolved.2},
                    "events": self.report,
                    "upd": self.upd,
                });
                let _ = std::fs::write(out, serde_json::to_string_pretty(&v).unwrap_or_default());
            }
        }

        fn trace(&self, msg: &str) {
            if self.args.dev || std::env::var_os("FASTKIWI_TRACE").is_some() {
                eprintln!("{msg}");
            }
        }

        fn on_host(&mut self, app: &str, epoch: u64, ev: HostEvent) {
            if self.hosts.get(app).map(|(e, _)| *e) != Some(epoch) {
                return; // from a tab that was closed in the meantime
            }
            match ev {
                HostEvent::Ready(h) => {
                    self.rec(json!({"ev": "handshake", "app": app, "port": h.port, "name": h.name, "version": h.version}));
                    if !self.modules.contains_key(app) {
                        self.build_module_view(
                            app,
                            ModuleSource::Process { port: h.port },
                            h.enter_url(),
                            epoch,
                        );
                    }
                }
                HostEvent::Failed(f) => {
                    self.rec(json!({"ev": "host-failed", "app": app, "why": format!("{f:?}")}));
                    self.trace(&format!("{app}: no valid handshake: {f:?}"));
                    self.hosts.remove(app);
                    self.send_ui(json!({"type": "module", "app": app, "status": "host-failed"}));
                }
                HostEvent::Exited(code) => {
                    self.rec(json!({"ev": "host-exited", "app": app, "code": code}));
                    self.trace(&format!("{app}: --hub-host exited ({code:?})"));
                    self.hosts.remove(app);
                    if self.modules.get(app).is_some_and(|m| m.epoch == epoch) {
                        self.modules.remove(app);
                    }
                    self.send_ui(json!({"type": "module", "app": app, "status": "host-exited"}));
                }
            }
        }

        /// The child WebView for a module: own profile, allowlist, no host objects, the
        /// embed.js marker (D-H18), web messages only from its own origin.
        fn build_module_view(&mut self, id: &str, source: ModuleSource, url: String, epoch: u64) {
            let mut ctx = WebContext::new(Some(data_dir(&format!("module-{id}"))));
            let (x, y, w, h) = self.bounds;
            let nav_id = id.to_string();
            let p_nav = self.proxy.clone();
            let p_ipc = self.proxy.clone();
            let p_load = self.proxy.clone();
            let ipc_id = id.to_string();
            let load_id = id.to_string();
            let (origin, port) = match &source {
                ModuleSource::Process { port } => (allowlist::process_origin(*port), Some(*port)),
                _ => (format!("https://{}/", allowlist::module_host(id)), None),
            };
            let mut b = WebViewBuilder::new_with_web_context(&mut ctx);
            if port.is_none() {
                let src = source.clone();
                let sid = id.to_string();
                b = b.with_custom_protocol(allowlist::MODULE_SCHEME.into(), move |_, req| {
                    serve_module(&sid, &src, req)
                });
            }
            let built = b
                .with_initialization_script(hosting::marker_script(id))
                .with_url(url)
                .with_bounds(Rect {
                    position: LogicalPosition::new(x, y).into(),
                    size: LogicalSize::new(w.max(1.0), h.max(1.0)).into(),
                })
                .with_navigation_handler(move |url| {
                    let ok = match port {
                        Some(p) => allowlist::process_nav_allowed(&url, p),
                        None => allowlist::module_nav_allowed(&url, &nav_id),
                    };
                    if !ok {
                        let _ = p_nav.send_event(Ev::Blocked {
                            who: nav_id.clone(),
                            host: allowlist::host_of(&url),
                        });
                    }
                    ok
                })
                .with_new_window_req_handler(|_, _| NewWindowResponse::Deny)
                .with_download_started_handler(|_, _| false)
                .with_permission_handler(|_| PermissionResponse::Deny)
                // No `.with_ipc_handler`: wry would then define a frozen `window.ipc` in the
                // hosted page, a name the module contract does not give modules (and that an
                // app's own page may use). Web messages are taken from WebView2 directly below.
                .with_on_page_load_handler(move |e, _| {
                    if matches!(e, PageLoadEvent::Finished) {
                        let _ = p_load.send_event(Ev::ModuleLoaded {
                            app: load_id.clone(),
                            epoch,
                        });
                    }
                })
                .with_devtools(self.args.dev)
                .with_hotkeys_zoom(true)
                .with_https_scheme(true)
                .with_browser_accelerator_keys(false)
                .with_additional_browser_args(browser_args())
                .build_as_child(&self.window);
            match built {
                Ok(webview) => {
                    harden(&webview, self.args.dev, false);
                    on_web_message(&webview, move |source, body| {
                        // Only messages from the module's own origin count.
                        if source.starts_with(&origin) {
                            let _ = p_ipc.send_event(Ev::ModuleIpc {
                                app: ipc_id.clone(),
                                body,
                            });
                        }
                    });
                    let _ = webview.set_visible(self.module_visible && self.shown_app == id);
                    self.send_ui(json!({"type": "module", "app": id, "status": "loading", "source": source.label()}));
                    self.modules.insert(
                        id.to_string(),
                        ModuleView {
                            webview,
                            _ctx: ctx,
                            epoch,
                            ready: false,
                            source,
                        },
                    );
                }
                Err(e) => {
                    self.send_ui(json!({"type": "module", "app": id, "status": "error", "error": e.to_string()}));
                }
            }
        }

        fn close_module(&mut self, id: &str) {
            self.modules.remove(id);
            if let Some((_, p)) = self.hosts.remove(id) {
                p.stop_with(hostproc::STOP_GRACE);
            }
        }

        /// Start `--hub-update [--check-only]` for `id`, if its copy declares it and no run
        /// for it is going.
        fn start_update(&mut self, id: &str, check_only: bool) {
            if self.runs.contains_key(id) {
                return;
            }
            let UpdateRoute::Hub(idx) = hosting::update_route(self.installs(id)) else {
                return;
            };
            let Some(exe) = self.installs(id)[idx].exe.clone() else {
                return;
            };
            self.run_seq += 1;
            let run = self.run_seq;
            let p = self.proxy.clone();
            let app = id.to_string();
            match hostproc::start_update(
                std::path::Path::new(&exe),
                check_only,
                self.window.hwnd(),
                hostproc::UpdateTiming::default(),
                move |ev| {
                    let _ = p.send_event(Ev::Update {
                        app: app.clone(),
                        run,
                        ev,
                    });
                },
            ) {
                Ok(handle) => {
                    self.runs.insert(
                        id.to_string(),
                        Run {
                            id: run,
                            check_only,
                            handle,
                        },
                    );
                    self.set_upd(id, json!({"state": "checking"}), check_only, true);
                }
                Err(e) => {
                    self.trace(&format!("{id}: --hub-update did not start: {e}"));
                    self.set_upd(
                        id,
                        json!({"state": "error", "code": "internal"}),
                        check_only,
                        false,
                    );
                }
            }
        }

        fn set_upd(&mut self, id: &str, mut v: Value, check_only: bool, running: bool) {
            if let Value::Object(o) = &mut v {
                o.remove("proto");
                o.insert("check_only".into(), Value::Bool(check_only));
                o.insert("running".into(), Value::Bool(running));
            }
            self.upd.insert(id.to_string(), v.clone());
            self.send_ui(json!({"type": "update", "app": id, "row": v}));
        }

        fn on_update(&mut self, app: &str, run: u64, ev: UpdateEvent) {
            let Some(check_only) = self
                .runs
                .get(app)
                .filter(|r| r.id == run)
                .map(|r| r.check_only)
            else {
                return;
            };
            let (line, running) = match ev {
                UpdateEvent::Line(l) => (l, true),
                UpdateEvent::End(end) => {
                    if let hostproc::UpdateEnd::Broken(why) = &end {
                        self.trace(&format!("{app}: --hub-update broke the contract: {why}"));
                        self.rec(json!({"ev": "update-broken", "app": app, "why": why}));
                    }
                    self.rec(json!({"ev": "update-end", "app": app, "check_only": check_only, "line": end.shown()}));
                    self.runs.remove(app);
                    (end.shown(), false)
                }
            };
            // A final line arrives twice (as a line, then as the end): forward it once.
            let forward = if running {
                !line.state.is_final()
            } else {
                true
            };
            if !check_only
                && forward
                && let Some(m) = self.modules.get(app).filter(|m| m.ready)
            {
                for msg in hosting::module_msgs(&line, true) {
                    post(&m.webview, &msg);
                }
            }
            let v = serde_json::to_value(&line).unwrap_or(Value::Null);
            // "running" ends only with the run itself, so a new run is never refused as busy.
            self.set_upd(app, v, check_only, running);
        }

        fn check_all(&mut self) {
            let ids: Vec<String> = self.apps.iter().map(|a| a.id.clone()).collect();
            for id in ids {
                self.start_update(&id, true);
            }
        }

        /// `--dev-host <id>=<exe>`: add that exe as a portable copy (dev only).
        fn add_dev_hosts(&self, d: &mut [AppDetection]) {
            for (id, exe) in &self.args.dev_hosts {
                let Some(x) = d.iter_mut().find(|x| &x.id == id) else {
                    continue;
                };
                let dir = exe.parent().map(PathBuf::from).unwrap_or_default();
                let hub = std::fs::read_to_string(dir.join(hubcli::APP_MANIFEST))
                    .ok()
                    .and_then(|t| hubcli::parse_app_manifest(&t, Some(id)).ok())
                    .map(|m| m.commands);
                x.installs.insert(
                    0,
                    detect::Install {
                        source: detect::Source::Portable,
                        version: detect::windows::file_version(&exe.display().to_string()),
                        size_bytes: std::fs::metadata(exe).ok().map(|m| m.len()),
                        location: dir.display().to_string(),
                        exe: Some(exe.display().to_string()),
                        aumid: None,
                        launchable: exe.is_file(),
                        web_entry: self
                            .app(id)
                            .and_then(|a| a.hub.as_ref())
                            .and_then(|h| h.module.as_ref())
                            .map(|m| dir.join(&m.web_dir).join(&m.entry))
                            .filter(|p| p.is_file())
                            .map(|p| p.display().to_string()),
                        found_by: "dev-host",
                        hub,
                    },
                );
                x.used_before = false;
            }
        }

        fn push_settings(&self, m: &ModuleView) {
            let (t, l, d) = self.resolved;
            for msg in protocol::settings_msgs(t, l, d) {
                post(&m.webview, &msg);
            }
        }

        fn on_module_msg(&mut self, app: &str, body: &str) {
            let parsed = protocol::parse_module_msg(body, app);
            match parsed {
                Ok(ModuleMsg::Ready { version, .. }) => {
                    if let Some(m) = self.modules.get_mut(app) {
                        m.ready = true;
                    }
                    if let Some(m) = self.modules.get(app) {
                        self.push_settings(m);
                    }
                    self.send_ui(json!({"type": "module", "app": app, "status": "ready", "module_version": version,
                        "source": self.modules.get(app).map(|m| m.source.label())}));
                    self.rec(json!({"ev": "ready", "app": app, "module_version": version}));
                    if self.args.dev_report.is_some() {
                        let p = self.proxy.clone();
                        let app = app.to_string();
                        std::thread::spawn(move || {
                            std::thread::sleep(Duration::from_millis(800));
                            let _ = p.send_event(Ev::Probe { app });
                        });
                    }
                }
                Ok(ModuleMsg::OpenOwnWindow {}) => {
                    let r = match self.first_launchable(app) {
                        Some(i) => self.launch(app, i),
                        None => Err("not installed".into()),
                    };
                    self.send_ui(json!({"type": "launched", "app": app, "ok": r.is_ok(), "error": r.err(), "from": "module"}));
                }
                Err(e) => {
                    self.rec(json!({"ev": "proto-error", "app": app, "error": e.to_string()}));
                    self.send_ui(
                        json!({"type": "proto-error", "app": app, "error": e.to_string()}),
                    );
                }
            }
        }

        fn layout_modules(&self) {
            let (x, y, w, h) = self.bounds;
            for m in self.modules.values() {
                let _ = m.webview.set_bounds(Rect {
                    position: LogicalPosition::new(x, y).into(),
                    size: LogicalSize::new(w.max(1.0), h.max(1.0)).into(),
                });
            }
        }

        fn on_page_msg(&mut self, page: Page, body: &str, flow: &mut ControlFlow) {
            if body.len() > 64 * 1024 {
                return;
            }
            let Ok(v) = serde_json::from_str::<Value>(body) else {
                return;
            };
            let s = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or("").to_string();
            if self.on_new_cmd(page, &s("cmd"), &v) {
                return;
            }
            // Everything below is the Hub window's own page.
            if page == Page::Bar && s("cmd") != "hello" {
                return;
            }
            match s("cmd").as_str() {
                "hello" => {
                    let st = self.state();
                    self.send_to(page, st);
                }
                "rescan" => {
                    self.send_ui(json!({"type": "scanning"}));
                    spawn_detection(self.apps.clone(), self.proxy.clone());
                }
                "launch" => {
                    let id = s("app");
                    let idx = v.get("install").and_then(Value::as_u64).unwrap_or(0) as usize;
                    let r = self.launch(&id, idx);
                    self.send_ui(
                        json!({"type": "launched", "app": id, "ok": r.is_ok(), "error": r.err()}),
                    );
                }
                "open-module" => {
                    let id = s("app");
                    for (k, m) in &self.modules {
                        if *k != id {
                            let _ = m.webview.set_visible(false);
                        }
                    }
                    self.open_module(&id);
                }
                "close-module" => self.close_module(&s("app")),
                "module-visible" => {
                    let vis = v.get("visible").and_then(Value::as_bool).unwrap_or(false);
                    let app = s("app");
                    self.module_visible = vis;
                    self.shown_app = app.clone();
                    for (k, m) in &self.modules {
                        let _ = m.webview.set_visible(vis && *k == app);
                    }
                }
                "module-bounds" => {
                    let f = |k: &str| v.get(k).and_then(Value::as_f64).unwrap_or(0.0).max(0.0);
                    self.bounds = (f("x"), f("y"), f("w"), f("h"));
                    self.layout_modules();
                }
                "settings" => {
                    if let Some(st) = v.get("settings") {
                        let old_hotkey = self.settings.hotkey.clone();
                        let old_history = self.settings.bar_history;
                        self.settings = Settings::from_json(&st.to_string());
                        if self.args.shot.is_none() {
                            let _ = settings::save(&self.settings);
                        }
                        if self.settings.hotkey != old_hotkey
                            && let Some(h) = &self.hotkey
                        {
                            h.rebind(&self.settings.hotkey);
                        }
                        if old_history && !self.settings.bar_history {
                            self.clear_history();
                        }
                        let st = self.state();
                        self.send_to(Page::Bar, st);
                    }
                    if let Some(r) = v.get("resolved") {
                        let t = serde_json::from_value(r["theme"].clone()).unwrap_or(Theme::Light);
                        let l = serde_json::from_value(r["lang"].clone()).unwrap_or(Lang::Ar);
                        let d = serde_json::from_value(r["digits"].clone()).unwrap_or(Digits::Arab);
                        self.resolved = (t, l, d);
                        for m in self.modules.values().filter(|m| m.ready) {
                            self.push_settings(m);
                        }
                    }
                }
                "update-check" => {
                    let id = s("app");
                    if id.is_empty() {
                        self.check_all();
                    } else {
                        self.start_update(&id, true);
                    }
                }
                "update-run" => self.start_update(&s("app"), false),
                "update-cancel" => {
                    if let Some(r) = self.runs.get(&s("app")) {
                        r.handle.cancel();
                    }
                }
                // Dev self-test only: press one of the stub module's own test buttons.
                "dev-click" if self.args.dev && self.args.shot.is_some() => {
                    let el = s("el");
                    if ["away", "bad", "own"].contains(&el.as_str())
                        && let Some(m) = self.modules.get(&s("app"))
                    {
                        let _ = m
                            .webview
                            .evaluate_script(&format!("document.getElementById('{el}')?.click()"));
                    }
                }
                "shot-ready" if self.args.shot.is_some() => {
                    let p = self.proxy.clone();
                    // An integration run waits for the page probe and its open-own-window.
                    let wait = if self.args.dev_report.is_some() {
                        3500
                    } else {
                        700
                    };
                    std::thread::spawn(move || {
                        std::thread::sleep(Duration::from_millis(wait));
                        let _ = p.send_event(Ev::ShotNow);
                    });
                }
                "quit" if self.args.shot.is_some() => *flow = ControlFlow::Exit,
                _ => {}
            }
        }

        fn on_event(&mut self, ev: Ev, flow: &mut ControlFlow) {
            match ev {
                Ev::HubIpc(body) => self.on_page_msg(Page::Hub, &body, flow),
                Ev::ModuleIpc { app, body } => self.on_module_msg(&app, &body),
                Ev::ModuleLoaded { app, epoch } => {
                    let p = self.proxy.clone();
                    std::thread::spawn(move || {
                        std::thread::sleep(READY_TIMEOUT);
                        let _ = p.send_event(Ev::ModuleTimeout { app, epoch });
                    });
                }
                Ev::ModuleTimeout { app, epoch } => {
                    if let Some(m) = self.modules.get(&app)
                        && m.epoch == epoch
                        && !m.ready
                    {
                        self.send_ui(json!({"type": "module", "app": app, "status": "no-proto", "source": m.source.label()}));
                        self.rec(json!({"ev": "no-proto", "app": app}));
                    }
                }
                Ev::Blocked { who, host } => {
                    self.rec(json!({"ev": "blocked", "app": who, "host": host}));
                    self.send_ui(json!({"type": "blocked", "app": who, "host": host}));
                }
                Ev::Detected(mut d) => {
                    self.add_dev_hosts(&mut d);
                    self.detection = Some(d);
                    // Online once per start (at most once a day thanks to the caches), and only
                    // while the setting is on: the signed catalog, icons, and release checks.
                    if self.settings.check_updates && !self.auto_checked && self.args.shot.is_none()
                    {
                        self.auto_checked = true;
                        self.check_all();
                        self.refresh_catalog();
                        self.fetch_icons();
                        self.run_checks(false);
                    }
                    let st = json!({"type": "detection", "detection": self.detection_json(), "mine": self.mine_json(), "find": self.find_exe().is_some()});
                    self.send_all(st);
                }
                Ev::Host { app, epoch, ev } => self.on_host(&app, epoch, ev),
                Ev::Update { app, run, ev } => self.on_update(&app, run, ev),
                Ev::Probe { app } => {
                    if let Some(m) = self.modules.get(&app) {
                        let p = self.proxy.clone();
                        let a = app.clone();
                        let _ = m
                            .webview
                            .evaluate_script_with_callback(PROBE_JS, move |json| {
                                let _ = p.send_event(Ev::ProbeResult {
                                    app: a.clone(),
                                    json,
                                });
                            });
                    }
                }
                Ev::ProbeResult { app, json } => {
                    // The callback gets the JSON of the script's value: here a JSON string.
                    let inner = serde_json::from_str::<String>(&json)
                        .ok()
                        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
                        .unwrap_or(Value::String(json));
                    self.rec(json!({"ev": "page", "app": app, "state": inner}));
                    // Then the page asks for its own window through its own adapter.
                    if let Some(m) = self.modules.get(&app) {
                        let _ = m.webview.evaluate_script(OPEN_OWN_JS);
                    }
                }
                Ev::BarIpc(body) => self.on_page_msg(Page::Bar, &body, flow),
                Ev::Hotkey => self.show_bar(),
                Ev::HotkeyStatus(st) => {
                    self.hotkey_status = serde_json::to_value(&st).unwrap_or(Value::Null);
                    self.send_all(json!({"type": "hotkey", "status": self.hotkey_status}));
                }
                Ev::Catalog(c) => {
                    self.catalog_serial = c.serial;
                    self.catalog_from_cache = true;
                    self.apps = catalog::visible(&c, self.args.dev);
                    spawn_detection(self.apps.clone(), self.proxy.clone());
                    self.fetch_icons();
                    let st = self.state();
                    self.send_all(st);
                }
                Ev::Checks { rows, instafind } => {
                    self.checking = false;
                    if instafind.is_some() {
                        self.instafind_version = instafind;
                    }
                    for (id, v) in rows {
                        self.checks.insert(id, v);
                    }
                    let st = self.state();
                    self.send_all(st);
                }
                Ev::IconsFetched => {
                    let st = self.state();
                    self.send_all(st);
                }
                Ev::FindDone { seq, hits } => {
                    let v = match &hits {
                        Ok(h) => json!({"type": "find", "seq": seq, "ok": true,
                            "hits": h.iter().map(|x| json!({"name": x.name, "dir": x.path.rsplit_once('\\').map(|(d, _)| d).unwrap_or(""), "is_dir": x.is_dir})).collect::<Vec<_>>()}),
                        Err(e) => json!({"type": "find", "seq": seq, "ok": false, "error": e}),
                    };
                    if let Ok(h) = hits {
                        self.last_hits = h;
                    }
                    self.send_all(v);
                }
                Ev::Ollama(model) => {
                    self.ollama = model;
                    self.send_all(json!({"type": "ollama", "ok": self.ollama.is_some()}));
                }
                Ev::AskDone { seq, answer } => {
                    let v = match answer {
                        Ok(t) => json!({"type": "ask", "seq": seq, "ok": true, "text": t}),
                        Err(_) => json!({"type": "ask", "seq": seq, "ok": false}),
                    };
                    self.send_all(v);
                }
                Ev::ShotNow => {
                    self.write_report();
                    if let Some(s) = &self.args.shot {
                        self.window.set_minimized(false);
                        match win::capture_client_png(self.window.hwnd(), &s.out) {
                            Ok((w, h)) => println!("shot {} {w}x{h}", s.out.display()),
                            Err(e) if self.shot_tries < 3 => {
                                self.shot_tries += 1;
                                eprintln!("shot retry: {e}");
                                let p = self.proxy.clone();
                                std::thread::spawn(move || {
                                    std::thread::sleep(Duration::from_millis(800));
                                    let _ = p.send_event(Ev::ShotNow);
                                });
                                return;
                            }
                            Err(e) => eprintln!("shot failed: {e}"),
                        }
                    }
                    *flow = ControlFlow::Exit;
                }
                Ev::ShotTimeout => {
                    self.rec(json!({"ev": "shot-timeout"}));
                    self.write_report();
                    eprintln!("shot timed out");
                    *flow = ControlFlow::Exit;
                }
            }
        }
    }

    /// The Hub's own page (`index.html`) or the bar's (`bar.html`): the only pages with wry's
    /// IPC handler, both on the Hub's own origin.
    fn page_builder<'a>(
        ctx: &'a mut WebContext,
        page: Page,
        proxy: &EventLoopProxy<Ev>,
        dev: bool,
    ) -> WebViewBuilder<'a> {
        let p_ipc = proxy.clone();
        let p_nav = proxy.clone();
        let url = match page {
            Page::Hub => allowlist::hub_start_url(),
            Page::Bar => format!("{}://hub/bar.html", allowlist::HUB_SCHEME),
        };
        WebViewBuilder::new_with_web_context(ctx)
            .with_custom_protocol(allowlist::HUB_SCHEME.into(), |_, req| serve_hub(req))
            .with_url(url)
            .with_ipc_handler(move |req: Request<String>| {
                if allowlist::hub_nav_allowed(&req.uri().to_string()) {
                    let body = req.into_body();
                    let _ = p_ipc.send_event(match page {
                        Page::Hub => Ev::HubIpc(body),
                        Page::Bar => Ev::BarIpc(body),
                    });
                }
            })
            .with_navigation_handler(move |url| {
                let ok = allowlist::hub_nav_allowed(&url);
                if !ok {
                    let _ = p_nav.send_event(Ev::Blocked {
                        who: "hub".into(),
                        host: allowlist::host_of(&url),
                    });
                }
                ok
            })
            .with_new_window_req_handler(|_, _| NewWindowResponse::Deny)
            .with_download_started_handler(|_, _| false)
            .with_permission_handler(|_| PermissionResponse::Deny)
            .with_devtools(dev)
            .with_hotkeys_zoom(false)
            .with_https_scheme(true)
            .with_browser_accelerator_keys(false)
            .with_default_context_menus(dev)
            .with_additional_browser_args(browser_args())
    }

    fn build_page(
        window: &tao::window::Window,
        ctx: &mut WebContext,
        page: Page,
        proxy: &EventLoopProxy<Ev>,
        dev: bool,
    ) -> wry::Result<WebView> {
        page_builder(ctx, page, proxy, dev).build(window)
    }

    const BAR_W: f64 = 680.0;
    const BAR_H: f64 = 460.0;

    /// The bar's own small window: no frame, on top, not in the taskbar, hidden until the
    /// shortcut is pressed.
    fn build_bar(
        el: &tao::event_loop::EventLoop<Ev>,
        ctx: &mut WebContext,
        proxy: &EventLoopProxy<Ev>,
        dev: bool,
    ) -> Option<(tao::window::Window, WebView)> {
        use tao::platform::windows::WindowBuilderExtWindows;
        let mut wb = WindowBuilder::new()
            .with_title("FastKiwi")
            .with_decorations(false)
            .with_always_on_top(true)
            .with_resizable(false)
            .with_visible(false)
            .with_skip_taskbar(true)
            .with_inner_size(LogicalSize::new(BAR_W, BAR_H));
        if let Ok(icon) = Icon::from_resource(1, None) {
            wb = wb.with_window_icon(Some(icon));
        }
        let w = wb.build(el).ok()?;
        let wv = build_page(&w, ctx, Page::Bar, proxy, dev).ok()?;
        harden(&wv, dev, true);
        Some((w, wv))
    }

    /// Run the online checks for these apps on a worker thread.
    struct CheckJob {
        id: String,
        repo: String,
        asset: Option<String>,
        allow_pre: bool,
        installed: Option<String>,
    }

    impl Hub {
        fn icon_for(&self, a: &CatApp) -> String {
            if let Some(name) = links::icon_file_name(&a.icon) {
                // The icons inside the exe are FastKiwi's; a third-party entry never gets one.
                if !a.third_party() && serve::ui_file(&format!("icons/{name}")).is_some() {
                    return format!("icons/{name}");
                }
                if let Some(d) = settings::hub_dir()
                    && let Some(p) = online::icon_path(&d, &a.id, &a.icon)
                    && p.is_file()
                    && let Some(f) = p.file_name()
                {
                    return format!("local-icons/{}", f.to_string_lossy());
                }
            }
            "icons/app.svg".into()
        }

        /// The site name a third-party entry would open (ASCII, punycode for IDN hosts).
        fn site_of(a: &CatApp) -> Option<String> {
            a.download
                .as_deref()
                .or(a.url.as_ref().map(|u| u.en.as_str()))
                .and_then(links::plain_https)
                .and_then(|u| u.host_str().map(str::to_string))
        }

        fn app_json(&self, a: &CatApp) -> Value {
            let h = a.hub.as_ref();
            json!({
                "id": a.id, "status": a.status, "name": a.name, "tagline": a.tagline,
                "description": a.description, "url": a.url.is_some(),
                "store": a.store.as_ref().map(|s| json!({"id": s.id, "live": s.live})),
                "platform": a.platform,
                "publisher": a.publisher, "third_party": a.third_party(),
                "site": if a.third_party() { Self::site_of(a) } else { None },
                "download": a.download.is_some(), "release": a.release.is_some(),
                "icon": self.icon_for(a),
                "kind": h.map(|h| h.kind), "size_mb": h.and_then(|h| h.size_mb),
                "on_demand": h.map(|h| &h.on_demand), "needs": h.and_then(|h| h.needs.as_ref()),
            })
        }

        fn mine_json(&self) -> Value {
            Value::Array(
                self.mine
                    .iter()
                    .map(|p| {
                        let icon = p
                            .icon
                            .as_ref()
                            .filter(|_| self.dir.is_some())
                            .map(|i| format!("local-icons/{i}"))
                            .unwrap_or_else(|| "icons/app.svg".into());
                        json!({"id": p.id, "name": p.name, "icon": icon,
                            "file": p.path.rsplit('\\').next().unwrap_or(""),
                            "exists": std::path::Path::new(&p.path).is_file()})
                    })
                    .collect(),
            )
        }

        /// The InstaFind exe whose command line the bar may use.
        fn find_exe(&self) -> Option<String> {
            if self
                .instafind_version
                .as_deref()
                .is_some_and(|v| !findcli::supports(v))
            {
                return None;
            }
            self.installs("instafind")
                .iter()
                .find(|i| i.source != detect::Source::Store && i.launchable)
                .and_then(|i| i.exe.clone())
        }

        fn show_bar(&mut self) {
            let Some((w, _)) = &self.bar else {
                return;
            };
            if let Some(m) = w.current_monitor().or_else(|| w.primary_monitor()) {
                let sf = m.scale_factor();
                let pos = m.position().to_logical::<f64>(sf);
                let size = m.size().to_logical::<f64>(sf);
                w.set_outer_position(LogicalPosition::new(
                    pos.x + (size.width - BAR_W) / 2.0,
                    pos.y + size.height * 0.16,
                ));
            }
            w.set_visible(true);
            w.set_focus();
            let st = self.state();
            self.send_bar(st);
            self.send_bar(json!({"type": "bar-show"}));
        }

        fn hide_bar(&mut self) {
            if let Some((w, _)) = &self.bar {
                w.set_visible(false);
            }
        }

        fn refresh_catalog(&self) {
            if !online::catalog_due(self.dir.as_deref(), updates::AUTO_MAX_AGE_SECS) {
                return;
            }
            let dir = self.dir.clone();
            let min = self.catalog_serial;
            let p = self.proxy.clone();
            std::thread::spawn(move || match online::refresh_catalog(dir.as_deref(), min) {
                Ok(Some(c)) => {
                    let _ = p.send_event(Ev::Catalog(Box::new(c)));
                }
                Ok(None) => {}
                Err(e) => {
                    if std::env::var_os("FASTKIWI_TRACE").is_some() {
                        eprintln!("catalog: {e}");
                    }
                }
            });
        }

        fn fetch_icons(&self) {
            let Some(dir) = self.dir.clone() else {
                return;
            };
            let todo: Vec<(String, String)> = self
                .apps
                .iter()
                .filter(|a| self.icon_for(a) == "icons/app.svg" && links::remote_icon(&a.icon).is_some())
                .map(|a| (a.id.clone(), a.icon.clone()))
                .collect();
            if todo.is_empty() {
                return;
            }
            let p = self.proxy.clone();
            std::thread::spawn(move || {
                for (id, icon) in todo {
                    online::fetch_icon(&dir, &id, &icon);
                }
                let _ = p.send_event(Ev::IconsFetched);
            });
        }

        /// Compare each installed FastKiwi app (and the Hub itself) with its newest GitHub
        /// release. `force`: the "Check now" button (the cache is reused for 10 minutes).
        fn run_checks(&mut self, force: bool) {
            if self.checking {
                return;
            }
            let mut jobs: Vec<CheckJob> = self
                .apps
                .iter()
                .filter(|a| !a.third_party() && a.platform == Platform::Windows)
                .filter_map(|a| {
                    let r = a.release.as_ref()?;
                    let inst = self.installs(&a.id).first()?;
                    Some(CheckJob {
                        id: a.id.clone(),
                        repo: r.repo.clone(),
                        asset: r.asset.clone(),
                        allow_pre: false,
                        installed: inst.version.clone(),
                    })
                })
                .collect();
            jobs.push(CheckJob {
                id: "fastkiwi-hub".into(),
                repo: links::HUB_REPO.into(),
                asset: Some("FastKiwi-Hub.exe".into()),
                // The Hub is a preview: its own pre-releases count.
                allow_pre: true,
                installed: Some(VERSION.into()),
            });
            let find = self
                .installs("instafind")
                .iter()
                .find(|i| i.source != detect::Source::Store && i.launchable)
                .and_then(|i| i.exe.clone())
                .filter(|_| self.instafind_version.is_none());
            self.checking = true;
            self.send_all(json!({"type": "checking"}));
            let dir = self.dir.clone();
            let p = self.proxy.clone();
            let age = if force {
                updates::MANUAL_MAX_AGE_SECS
            } else {
                updates::AUTO_MAX_AGE_SECS
            };
            std::thread::spawn(move || {
                let instafind = find.as_deref().and_then(findcli::version);
                let mut rows = Vec::new();
                for j in jobs {
                    let installed = if j.id == "instafind" {
                        j.installed.or_else(|| instafind.clone())
                    } else {
                        j.installed
                    };
                    let v = match online::latest(
                        dir.as_deref(),
                        &j.repo,
                        j.asset.as_deref(),
                        j.allow_pre,
                        age,
                    ) {
                        Ok(l) => {
                            let mut v = serde_json::to_value(updates::compare(installed.as_deref(), l))
                                .unwrap_or(Value::Null);
                            if let Value::Object(o) = &mut v {
                                o.insert("installed".into(), json!(installed));
                            }
                            v
                        }
                        Err(()) => serde_json::to_value(Check::Unreachable).unwrap_or(Value::Null),
                    };
                    rows.push((j.id, v));
                }
                let _ = p.send_event(Ev::Checks { rows, instafind });
            });
        }

        fn lang_text<'a>(&self, t: &'a catalog::Text, v: &Value) -> &'a str {
            if v.get("lang").and_then(Value::as_str) == Some("en") {
                &t.en
            } else {
                &t.ar
            }
        }

        /// The URL behind an app's button, from the catalog or the last update check.
        fn link_for(&self, id: &str, what: &str, v: &Value) -> Option<String> {
            let a = self.app(id)?;
            match what {
                "page" => a.url.as_ref().map(|u| self.lang_text(u, v).to_string()),
                "download" => a.download.clone(),
                "store" => a
                    .store
                    .as_ref()
                    .filter(|s| s.live)
                    .and_then(|s| links::store_page(&s.id)),
                "update" if !a.third_party() => {
                    let l = self.checks.get(id)?.get("latest")?;
                    l.get("download")
                        .and_then(Value::as_str)
                        .or_else(|| l.get("page").and_then(Value::as_str))
                        .map(str::to_string)
                }
                _ => None,
            }
            .filter(|u| {
                a.may_open(u)
                    || (what == "update"
                        && a.release.as_ref().is_some_and(|r| {
                            u.starts_with(&links::release_asset_prefix(&r.repo))
                                || u.starts_with(&format!("{}/", links::releases_page(&r.repo)))
                        }))
            })
        }

        fn hub_link(&self, what: &str) -> Option<String> {
            Some(match what {
                "support" => links::SUPPORT.into(),
                "site" => links::SITE.into(),
                "releases" => links::HUB_RELEASES.into(),
                "source" => "https://github.com/sarmadiq9090-coder/fastkiwi-hub".into(),
                "adding" => "https://github.com/sarmadiq9090-coder/fastkiwi-hub/blob/main/docs/ADDING-AN-APP.md".into(),
                "update" => {
                    let l = self.checks.get("fastkiwi-hub").and_then(|c| c.get("latest"));
                    l.and_then(|l| l.get("download").or_else(|| l.get("page")))
                        .and_then(Value::as_str)
                        .filter(|u| u.starts_with(&format!("{}/", links::HUB_RELEASES)))
                        .map(str::to_string)
                        .unwrap_or_else(|| links::HUB_RELEASES.into())
                }
                _ => return None,
            })
        }

        fn open_link(&mut self, page: Page, v: &Value) {
            let s = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or("").to_string();
            let (id, what) = (s("app"), s("what"));
            let Some(url) = self.link_for(&id, &what, v) else {
                self.send_to(page, json!({"type": "link", "ok": false}));
                return;
            };
            if self.app(&id).is_some_and(|a| a.third_party()) {
                let host = links::plain_https(&url)
                    .and_then(|u| u.host_str().map(str::to_string))
                    .unwrap_or_default();
                let confirmed = v.get("confirm").and_then(Value::as_bool) == Some(true);
                if !self.settings.trusted_sites.contains(&host) {
                    if !confirmed {
                        self.send_to(page, json!({"type": "confirm-site", "app": id, "what": what, "site": host}));
                        return;
                    }
                    self.settings.trusted_sites.push(host);
                    self.settings.trusted_sites.sort();
                    if self.dir.is_some() {
                        let _ = settings::save(&self.settings);
                    }
                }
            }
            let r = if self.args.shot.is_some() {
                Ok(()) // screenshot runs never open the browser
            } else {
                win::open_url(&url)
            };
            self.send_to(page, json!({"type": "link", "ok": r.is_ok()}));
        }

        fn add_program(&mut self) {
            let Some(dir) = self.dir.clone() else {
                return;
            };
            let ar = self.resolved.1 == Lang::Ar;
            let (title, label) = if ar {
                ("أضف برنامجًا من جهازك", "البرامج والاختصارات")
            } else {
                ("Add a program from your PC", "Programs and shortcuts")
            };
            let Some(path) = win::pick_program(self.window.hwnd(), title, label) else {
                return;
            };
            let name = if path.to_ascii_lowercase().ends_with(".exe") {
                win::file_description(&path)
            } else {
                None
            };
            let r = mine::add(&mut self.mine, &path, name);
            match r {
                Ok(p) => {
                    let icon = mine::icon_name(&p.id);
                    if win::save_file_icon(&path, &dir.join("icons").join(&icon)).is_ok()
                        && let Some(e) = self.mine.iter_mut().find(|x| x.id == p.id)
                    {
                        e.icon = Some(icon);
                    }
                    let _ = mine::save(&dir, &self.mine);
                    self.send_ui(json!({"type": "mine-added", "name": p.name}));
                }
                Err(e) => self.send_ui(json!({"type": "mine-error", "error": e})),
            }
            let st = self.state();
            self.send_all(st);
        }

        fn remove_program(&mut self, id: &str) {
            let Some(dir) = self.dir.clone() else {
                return;
            };
            if let Some(pos) = self.mine.iter().position(|p| p.id == id) {
                let p = self.mine.remove(pos);
                if let Some(i) = p.icon {
                    let _ = std::fs::remove_file(dir.join("icons").join(i));
                }
                let _ = mine::save(&dir, &self.mine);
            }
            let st = self.state();
            self.send_all(st);
        }

        fn clear_history(&mut self) {
            self.history.clear();
        }

        /// Commands both pages may send. `true` when handled.
        fn on_new_cmd(&mut self, page: Page, cmd: &str, v: &Value) -> bool {
            let s = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or("").to_string();
            let seq = v.get("seq").and_then(Value::as_u64).unwrap_or(0);
            match cmd {
                "open-link" => self.open_link(page, v),
                "hub-link" => {
                    if let Some(u) = self.hub_link(&s("what"))
                        && self.args.shot.is_none()
                    {
                        let _ = win::open_url(&u);
                    }
                }
                "check-updates" => self.run_checks(true),
                "add-program" if page == Page::Hub => self.add_program(),
                "remove-program" if page == Page::Hub => self.remove_program(&s("id")),
                "launch-mine" => {
                    let id = s("id");
                    let r = match self.mine.iter().find(|p| p.id == id) {
                        Some(p) if self.args.shot.is_none() => {
                            win::launch(&Target::Picked(p.path.clone()))
                        }
                        Some(_) => Ok(()),
                        None => Err("not found".into()),
                    };
                    let name = self.mine.iter().find(|p| p.id == id).map(|p| p.name.clone());
                    self.send_to(page, json!({"type": "launched", "app": id, "name": name, "ok": r.is_ok(), "error": r.err()}));
                    if page == Page::Bar {
                        self.hide_bar();
                    }
                }
                "bar-launch" => {
                    let id = s("app");
                    let r = match self.first_launchable(&id) {
                        Some(i) => self.launch(&id, i),
                        None => Err("not installed".into()),
                    };
                    let ok = r.is_ok();
                    self.send_to(page, json!({"type": "launched", "app": id, "ok": ok, "error": r.err()}));
                    if page == Page::Bar && ok {
                        self.hide_bar();
                    }
                }
                "bar-find" => {
                    let Some(exe) = self.find_exe() else {
                        self.send_to(page, json!({"type": "find", "seq": seq, "ok": false, "error": "no-instafind"}));
                        return true;
                    };
                    let q = s("q");
                    let known = self.instafind_version.clone();
                    let p = self.proxy.clone();
                    std::thread::spawn(move || {
                        let version = known.clone().or_else(|| findcli::version(&exe));
                        if known.is_none() {
                            let _ = p.send_event(Ev::Checks {
                                rows: Vec::new(),
                                instafind: Some(version.clone().unwrap_or_else(|| "0".into())),
                            });
                        }
                        let hits = match version.as_deref() {
                            Some(v) if findcli::supports(v) => findcli::search(&exe, &q, 8),
                            _ => Err("old-instafind".into()),
                        };
                        let _ = p.send_event(Ev::FindDone { seq, hits });
                    });
                }
                "bar-open" => {
                    let i = v.get("i").and_then(Value::as_u64).unwrap_or(u64::MAX) as usize;
                    let open = v.get("open").and_then(Value::as_bool).unwrap_or(false);
                    if let Some(h) = self.last_hits.get(i).cloned() {
                        let _ = win::open_found(&h.path, open);
                        if page == Page::Bar {
                            self.hide_bar();
                        }
                    }
                }
                "ollama-probe" => {
                    if self
                        .ollama_probed
                        .is_none_or(|t| t.elapsed() > Duration::from_secs(30))
                    {
                        self.ollama_probed = Some(Instant::now());
                        let p = self.proxy.clone();
                        std::thread::spawn(move || {
                            let _ = p.send_event(Ev::Ollama(online::ollama_model()));
                        });
                    }
                }
                "bar-ask" => {
                    let Some(model) = self.ollama.clone() else {
                        self.send_to(page, json!({"type": "ask", "seq": seq, "ok": false}));
                        return true;
                    };
                    let q = s("q");
                    let p = self.proxy.clone();
                    std::thread::spawn(move || {
                        let answer = online::ollama_ask(&model, &q);
                        let _ = p.send_event(Ev::AskDone { seq, answer });
                    });
                }
                "bar-hide" => self.hide_bar(),
                "show-hub" => {
                    self.hide_bar();
                    self.window.set_minimized(false);
                    self.window.set_visible(true);
                    self.window.set_focus();
                }
                "history-add" => {
                    let q: String = s("q").trim().chars().take(200).collect();
                    if self.settings.bar_history && !q.is_empty() {
                        self.history.retain(|h| h != &q);
                        self.history.insert(0, q);
                        self.history.truncate(20);
                    }
                }
                "history-clear" => {
                    self.clear_history();
                    let st = self.state();
                    self.send_all(st);
                }
                _ => return false,
            }
            true
        }
    }

    pub fn run() {
        let args = parse_args();
        SHOT_MODE.store(args.shot.is_some(), std::sync::atomic::Ordering::Relaxed);
        if let Some(out) = &args.detect_json {
            write_detect_json(&args, out);
            return;
        }
        if wry::webview_version().is_err() {
            win::webview2_missing();
            return;
        }
        let event_loop = EventLoopBuilder::<Ev>::with_user_event().build();
        let proxy = event_loop.create_proxy();
        let mut wb = WindowBuilder::new()
            .with_title("FastKiwi Hub")
            .with_inner_size(LogicalSize::new(1366.0, 768.0))
            .with_min_inner_size(LogicalSize::new(720.0, 520.0));
        if let Ok(icon) = Icon::from_resource(1, None) {
            wb = wb.with_window_icon(Some(icon));
        }
        if args.shot.is_some() {
            // Off screen and never focused: screenshots must not get in the way of whoever is
            // using the PC. PrintWindow captures it there.
            wb = wb
                .with_position(LogicalPosition::new(-4000.0, 40.0))
                .with_focused(false)
                .with_resizable(false);
        }
        let window = match wb.build(&event_loop) {
            Ok(w) => w,
            Err(e) => {
                eprintln!("window: {e}");
                return;
            }
        };
        if args.shot.is_some() {
            window.set_outer_position(LogicalPosition::new(-4000.0, 40.0));
        }
        if std::env::var_os("FASTKIWI_TRACE").is_some() {
            eprintln!("window at {:?}", window.outer_position());
        }

        let mut hub_ctx = WebContext::new(Some(data_dir("hub")));
        let hub = build_page(&window, &mut hub_ctx, Page::Hub, &proxy, args.dev);
        let hub = match hub {
            Ok(h) => h,
            Err(e) => {
                eprintln!("WebView2: {e}");
                return;
            }
        };
        harden(&hub, args.dev, true);

        // Screenshot runs write nothing to the Hub's folder.
        let dir = if args.shot.is_some() {
            None
        } else {
            settings::hub_dir()
        };
        let (apps, catalog_serial, catalog_from_cache) = load_catalog(&args, dir.as_deref());
        spawn_detection(apps.clone(), proxy.clone());
        let settings_now = if args.shot.is_some() {
            Settings::default()
        } else {
            settings::load()
        };
        // The bar window (hidden until the shortcut) and the shortcut itself.
        let (bar, hk) = if args.shot.is_some() {
            (None, None)
        } else {
            let bar = build_bar(&event_loop, &mut hub_ctx, &proxy, args.dev);
            let (p1, p2) = (proxy.clone(), proxy.clone());
            let hk = Hotkey::start(
                &settings_now.hotkey,
                move || {
                    let _ = p1.send_event(Ev::Hotkey);
                },
                move |st| {
                    let _ = p2.send_event(Ev::HotkeyStatus(st));
                },
            );
            (bar, hk)
        };
        if args.shot.is_some() {
            let p = proxy.clone();
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_secs(25));
                let _ = p.send_event(Ev::ShotTimeout);
            });
        }
        let mut state = Hub {
            mine: dir.as_deref().map(mine::load).unwrap_or_default(),
            history: Vec::new(),
            dir,
            catalog_serial,
            catalog_from_cache,
            checks: HashMap::new(),
            checking: false,
            instafind_version: None,
            bar,
            hotkey: hk,
            hotkey_status: json!({"state": "off"}),
            last_hits: Vec::new(),
            ollama: None,
            ollama_probed: None,
            _ctx: hub_ctx,
            settings: settings_now,
            args,
            window,
            hub,
            apps,
            detection: None,
            resolved: (Theme::Light, Lang::Ar, Digits::Arab),
            modules: HashMap::new(),
            epoch: 0,
            bounds: (0.0, 92.0, 1366.0, 676.0),
            module_visible: false,
            shown_app: String::new(),
            last_launch: HashMap::new(),
            proxy,
            hosts: HashMap::new(),
            runs: HashMap::new(),
            run_seq: 0,
            upd: HashMap::new(),
            auto_checked: false,
            shot_tries: 0,
            report: Vec::new(),
            started: Instant::now(),
        };

        event_loop.run(move |event, _, flow| {
            if *flow != ControlFlow::Exit {
                *flow = ControlFlow::Wait;
            }
            if std::env::var_os("FASTKIWI_TRACE").is_some() {
                eprintln!("event: {event:?}");
            }
            match event {
                Event::UserEvent(ev) => state.on_event(ev, flow),
                Event::WindowEvent {
                    event: WindowEvent::CloseRequested,
                    window_id,
                    ..
                } => {
                    if state.bar.as_ref().is_some_and(|(w, _)| w.id() == window_id) {
                        state.hide_bar();
                    } else {
                        *flow = ControlFlow::Exit;
                    }
                }
                Event::WindowEvent {
                    event: WindowEvent::Focused(false),
                    window_id,
                    ..
                } if state.bar.as_ref().is_some_and(|(w, _)| w.id() == window_id) => {
                    state.hide_bar();
                }
                _ => {}
            }
        });
    }
}
