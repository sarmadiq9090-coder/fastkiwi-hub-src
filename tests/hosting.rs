//! Phase D host side, without processes: which copies declare hub commands, which copy the Hub
//! drives, how update progress reaches a hosted module, and the embed.js marker.

use hubcli::{Command, UpdateCode, UpdateLine, UpdateState};
use kiwi_hub::catalog;
use kiwi_hub::detect::{self, AppDetection, Install, Source, fixture::FixtureView};
use kiwi_hub::hosting::{self, UpdateRoute};
use kiwi_hub::protocol::{self, HubMsg, UpdateState as Mod};

fn detected() -> (Vec<catalog::App>, Vec<AppDetection>) {
    let path = format!(
        "{}/tests/fixtures/detect-hub-caps.json",
        env!("CARGO_MANIFEST_DIR")
    );
    let view = FixtureView::from_json(&std::fs::read_to_string(path).unwrap()).unwrap();
    let apps = catalog::visible(&catalog::parse(catalog::FIXTURE).unwrap(), false);
    let d = detect::detect_all(&view, &apps);
    (apps, d)
}

fn installs<'a>(d: &'a [AppDetection], id: &str) -> &'a [Install] {
    &d.iter().find(|x| x.id == id).unwrap().installs
}

fn app<'a>(a: &'a [catalog::App], id: &str) -> &'a catalog::App {
    a.iter().find(|x| x.id == id).unwrap()
}

#[test]
fn capability_file_is_read_per_copy() {
    let (apps, d) = detected();

    let lemon = installs(&d, "lemon");
    assert_eq!(lemon.len(), 1);
    assert_eq!(
        lemon[0].hub.as_deref(),
        Some(&[Command::HubHost, Command::HubUpdate, Command::ExportData][..])
    );
    assert_eq!(hosting::host_install(app(&apps, "lemon"), lemon), Some(0));
    assert_eq!(hosting::update_route(lemon), UpdateRoute::Hub(0));

    // A file that names another app counts as no file: own window only.
    let find = installs(&d, "instafind");
    assert_eq!(find.len(), 1);
    assert_eq!(find[0].hub, None);
    assert_eq!(hosting::host_install(app(&apps, "instafind"), find), None);
    assert_eq!(hosting::update_route(find), UpdateRoute::OwnApp);

    // InstaWords: the Store copy declares it but is not driven before spike S1; the Inno copy's
    // file is broken (trailing comma). Kind S is never started with --hub-host.
    let words = installs(&d, "instawords");
    assert_eq!(words.len(), 2);
    let store = words.iter().find(|i| i.source == Source::Store).unwrap();
    let inno = words.iter().find(|i| i.source == Source::Inno).unwrap();
    assert_eq!(
        store.hub.as_deref(),
        Some(&[Command::HubUpdate, Command::ExportData][..])
    );
    assert_eq!(inno.hub, None);
    assert_eq!(hosting::host_install(app(&apps, "instawords"), words), None);
    assert_eq!(hosting::update_route(words), UpdateRoute::Store);

    // Remember Me (kind L): updates in place, never hosted. Its file starts with a BOM.
    let rm = installs(&d, "remember-me");
    assert!(rm[0].declares(Command::HubUpdate));
    assert!(!rm[0].declares(Command::HubHost));
    assert_eq!(hosting::host_install(app(&apps, "remember-me"), rm), None);
    assert_eq!(hosting::update_route(rm), UpdateRoute::Hub(0));
}

#[test]
fn update_routes() {
    assert_eq!(hosting::update_route(&[]), UpdateRoute::NotInstalled);
    assert_eq!(
        serde_json::to_value(UpdateRoute::Hub(1)).unwrap(),
        serde_json::json!({"route": "hub", "install": 1})
    );
    assert_eq!(
        serde_json::to_value(UpdateRoute::Store).unwrap(),
        serde_json::json!({"route": "store"})
    );
    assert_eq!(
        serde_json::to_value(UpdateRoute::OwnApp).unwrap(),
        serde_json::json!({"route": "own-app"})
    );
}

#[test]
fn a_missing_exe_is_never_driven() {
    let (apps, mut d) = detected();
    let lemon = d.iter_mut().find(|x| x.id == "lemon").unwrap();
    lemon.installs[0].launchable = false;
    assert_eq!(
        hosting::host_install(app(&apps, "lemon"), &lemon.installs),
        None
    );
    assert_eq!(hosting::update_route(&lemon.installs), UpdateRoute::OwnApp);
}

fn line(state: UpdateState, progress: Option<f64>) -> UpdateLine {
    UpdateLine {
        progress,
        ..UpdateLine::new(state)
    }
}

#[test]
fn update_progress_reaches_the_module() {
    let up = |state: Mod, progress: Option<f64>| HubMsg::UpdateProgress { state, progress };
    assert_eq!(
        hosting::module_msgs(&line(UpdateState::Checking, None), true),
        vec![up(Mod::Checking, None)]
    );
    assert_eq!(
        hosting::module_msgs(&line(UpdateState::Downloading, Some(0.45)), true),
        vec![up(Mod::Downloading, Some(0.45))]
    );
    assert_eq!(
        hosting::module_msgs(&line(UpdateState::Installing, Some(0.9)), false),
        vec![up(Mod::Installing, Some(0.9))]
    );
    assert_eq!(
        hosting::module_msgs(&line(UpdateState::Done, None), true),
        vec![up(Mod::Done, None), up(Mod::RestartNeeded, None)]
    );
    assert_eq!(
        hosting::module_msgs(&line(UpdateState::Done, None), false),
        vec![up(Mod::Done, None)]
    );
    let err = UpdateLine {
        code: Some(UpdateCode::Offline),
        ..UpdateLine::new(UpdateState::Error)
    };
    assert_eq!(
        hosting::module_msgs(&err, true),
        vec![up(Mod::Failed, None)]
    );
    assert!(hosting::module_msgs(&line(UpdateState::None, None), true).is_empty());
    // Every message the mapping makes passes the Hub's own module-protocol validation.
    for s in [
        UpdateState::Checking,
        UpdateState::Downloading,
        UpdateState::Installing,
        UpdateState::Done,
        UpdateState::Error,
    ] {
        let p = matches!(s, UpdateState::Downloading | UpdateState::Installing).then_some(0.5);
        for m in hosting::module_msgs(&line(s, p), true) {
            protocol::parse_hub_msg(&protocol::encode_hub_msg(&m)).unwrap();
        }
    }
}

#[test]
fn embed_marker() {
    let js = hosting::marker_script("lemon");
    assert!(js.contains("\"__FASTKIWI_HUB__\""));
    assert!(js.contains("proto:\"fastkiwi-module@1\""));
    assert!(js.contains("module:\"lemon\""));
    assert!(js.contains("writable:false"));
    // The marker must match what sdk/embed.js looks for.
    let embed =
        std::fs::read_to_string(format!("{}/sdk/embed.js", env!("CARGO_MANIFEST_DIR"))).unwrap();
    assert!(embed.contains("window.__FASTKIWI_HUB__"));
    assert!(embed.contains("marker.proto === PROTO"));
    assert!(embed.contains("marker.module === MODULE"));
}
