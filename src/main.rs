//! FastKiwi.exe: the Hub window (Phase D prototype).
//!
//! One tao window. The Hub UI is one WebView2 served over the `fastkiwi` custom protocol (no
//! local HTTP server). A hosted module gets its own child WebView2 with its own user-data
//! folder, a navigation allowlist, no host objects, and only `fastkiwi-module@1` web messages:
//! - kind S (web folder): its own origin `fkmod.<id>`, served by the Hub;
//! - kind P (Phase D): the app's own `--hub-host` page on `127.0.0.1:<port>` (`hostproc`).
//!
//! Detection is read only. Standalone launches are ShellExecute and forget. Updates run the
//! app's own `--hub-update` (fastkiwi-hub-cli@1). The Hub itself has no network, install or
//! uninstall code.

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
    use kiwi_hub::catalog::{self, App as CatApp, Kind};
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
                if is_hub {
                    let _ = s.SetIsZoomControlEnabled(false);
                }
            }
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

    fn load_catalog(args: &Args) -> Vec<CatApp> {
        let text = args
            .catalog
            .as_ref()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .unwrap_or_else(|| catalog::FIXTURE.to_string());
        match catalog::parse(&text) {
            Ok(c) => catalog::visible(&c, args.dev),
            Err(e) => {
                eprintln!("catalog: {e}");
                Vec::new()
            }
        }
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
        let apps = load_catalog(args);
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
            if let Some(entry) = self.installs(id).iter().find_map(|i| i.web_entry.clone()) {
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
            let apps: Vec<Value> = self
                .apps
                .iter()
                .map(|a| {
                    let h = a.hub.as_ref();
                    json!({
                        "id": a.id, "status": a.status, "name": a.name, "tagline": a.tagline,
                        "icon": a.icon, "store": a.store, "winget": a.winget,
                        "kind": h.map(|h| h.kind), "size_mb": h.map(|h| h.size_mb),
                        "on_demand": h.map(|h| &h.on_demand), "needs": h.and_then(|h| h.needs.as_ref()),
                        "data_dir": h.map(|h| &h.data_dir),
                    })
                })
                .collect();
            json!({
                "type": "state",
                "version": VERSION,
                "dev": self.args.dev,
                "apps": apps,
                "detection": self.detection_json(),
                "settings": self.settings,
                "upd": self.upd,
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
            win::launch(&target)
        }

        fn first_launchable(&self, id: &str) -> Option<usize> {
            self.installs(id).iter().position(|i| i.launchable)
        }

        /// The copy of a kind P app to run with `--hub-host`, if any declares it.
        fn host_install(&self, id: &str) -> Option<usize> {
            hosting::host_install(self.app(id)?, self.installs(id))
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
                    self.trace(&format!("{app}: no valid handshake: {f:?}"));
                    self.hosts.remove(app);
                    self.send_ui(json!({"type": "module", "app": app, "status": "host-failed"}));
                }
                HostEvent::Exited(code) => {
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
                .with_ipc_handler(move |req: Request<String>| {
                    // Only messages from the module's own origin count.
                    if req.uri().to_string().starts_with(&origin) {
                        let _ = p_ipc.send_event(Ev::ModuleIpc {
                            app: ipc_id.clone(),
                            body: req.into_body(),
                        });
                    }
                })
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
                    }
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
                        version: None,
                        size_bytes: std::fs::metadata(exe).ok().map(|m| m.len()),
                        location: dir.display().to_string(),
                        exe: Some(exe.display().to_string()),
                        aumid: None,
                        launchable: exe.is_file(),
                        web_entry: None,
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
                }
                Ok(ModuleMsg::OpenOwnWindow {}) => {
                    let r = match self.first_launchable(app) {
                        Some(i) => self.launch(app, i),
                        None => Err("not installed".into()),
                    };
                    self.send_ui(json!({"type": "launched", "app": app, "ok": r.is_ok(), "error": r.err(), "from": "module"}));
                }
                Err(e) => {
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

        fn on_hub_msg(&mut self, body: &str, flow: &mut ControlFlow) {
            if body.len() > 64 * 1024 {
                return;
            }
            let Ok(v) = serde_json::from_str::<Value>(body) else {
                return;
            };
            let s = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or("").to_string();
            match s("cmd").as_str() {
                "hello" => self.send_ui(self.state()),
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
                        self.settings = Settings::from_json(&st.to_string());
                        if self.args.shot.is_none() {
                            let _ = settings::save(&self.settings);
                        }
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
                    std::thread::spawn(move || {
                        std::thread::sleep(Duration::from_millis(700));
                        let _ = p.send_event(Ev::ShotNow);
                    });
                }
                "quit" if self.args.shot.is_some() => *flow = ControlFlow::Exit,
                _ => {}
            }
        }

        fn on_event(&mut self, ev: Ev, flow: &mut ControlFlow) {
            match ev {
                Ev::HubIpc(body) => self.on_hub_msg(&body, flow),
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
                    }
                }
                Ev::Blocked { who, host } => {
                    self.send_ui(json!({"type": "blocked", "app": who, "host": host}));
                }
                Ev::Detected(mut d) => {
                    self.add_dev_hosts(&mut d);
                    self.detection = Some(d);
                    self.send_ui(json!({"type": "detection", "detection": self.detection_json()}));
                    // The update check runs once per start, and only if the user turned it on.
                    if self.settings.check_updates && !self.auto_checked && self.args.shot.is_none()
                    {
                        self.auto_checked = true;
                        self.check_all();
                    }
                }
                Ev::Host { app, epoch, ev } => self.on_host(&app, epoch, ev),
                Ev::Update { app, run, ev } => self.on_update(&app, run, ev),
                Ev::ShotNow => {
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
                    eprintln!("shot timed out");
                    *flow = ControlFlow::Exit;
                }
            }
        }
    }

    pub fn run() {
        let args = parse_args();
        SHOT_MODE.store(args.shot.is_some(), std::sync::atomic::Ordering::Relaxed);
        if let Some(out) = &args.detect_json {
            write_detect_json(&args, out);
            return;
        }
        let event_loop = EventLoopBuilder::<Ev>::with_user_event().build();
        let proxy = event_loop.create_proxy();
        let mut wb = WindowBuilder::new()
            .with_title("FastKiwi")
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
        let p_ipc = proxy.clone();
        let p_nav = proxy.clone();
        let hub = WebViewBuilder::new_with_web_context(&mut hub_ctx)
            .with_custom_protocol(allowlist::HUB_SCHEME.into(), |_, req| serve_hub(req))
            .with_url(allowlist::hub_start_url())
            .with_ipc_handler(move |req: Request<String>| {
                if allowlist::hub_nav_allowed(&req.uri().to_string()) {
                    let _ = p_ipc.send_event(Ev::HubIpc(req.into_body()));
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
            .with_devtools(args.dev)
            .with_hotkeys_zoom(false)
            .with_https_scheme(true)
            .with_browser_accelerator_keys(false)
            .with_default_context_menus(args.dev)
            .with_additional_browser_args(browser_args())
            .build(&window);
        let hub = match hub {
            Ok(h) => h,
            Err(e) => {
                eprintln!("WebView2: {e}");
                return;
            }
        };
        harden(&hub, args.dev, true);

        let apps = load_catalog(&args);
        spawn_detection(apps.clone(), proxy.clone());
        if args.shot.is_some() {
            let p = proxy.clone();
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_secs(25));
                let _ = p.send_event(Ev::ShotTimeout);
            });
        }
        let mut state = Hub {
            settings: if args.shot.is_some() {
                Settings::default()
            } else {
                settings::load()
            },
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
                    ..
                } => *flow = ControlFlow::Exit,
                _ => {}
            }
            let _ = &hub_ctx;
        });
    }
}
