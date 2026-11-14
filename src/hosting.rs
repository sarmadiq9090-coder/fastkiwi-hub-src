//! Phase D rules for which copy of an app the Hub drives, and how update progress reaches a
//! hosted module. Pure functions over the catalog and detection, so they are tested without
//! processes (the processes are `hostproc`).

use crate::catalog::{App, Kind};
use crate::detect::{Install, Source};
use crate::protocol::{HubMsg, UpdateState as ModState};
use hubcli::{Command, UpdateLine, UpdateState};

/// A copy the Hub may run hub commands on: found by detection with an exe path, present, and
/// not a Store copy. Store copies need their app execution alias for package identity (spike
/// S1, before Phase E), so for now they open in their own window and update in the Store.
fn drivable(i: &Install) -> bool {
    i.source != Source::Store && i.launchable && i.exe.is_some()
}

/// The copy to host with `--hub-host`: a kind P app whose copy declares `hub-host`.
pub fn host_install(app: &App, installs: &[Install]) -> Option<usize> {
    (app.hub.as_ref()?.kind == Kind::P)
        .then(|| {
            installs
                .iter()
                .position(|i| drivable(i) && i.declares(Command::HubHost))
        })
        .flatten()
}

/// How an installed app is updated, as the Updates screen shows it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "kebab-case", tag = "route", content = "install")]
pub enum UpdateRoute {
    /// In place, through the copy's own `--hub-update`.
    Hub(usize),
    /// A Store copy: in the Store for now (in place after spike S1/S2, Phase E).
    Store,
    /// An older build without `--hub-update`: it updates itself from its own window.
    OwnApp,
    NotInstalled,
}

pub fn update_route(installs: &[Install]) -> UpdateRoute {
    if let Some(i) = installs
        .iter()
        .position(|i| drivable(i) && i.declares(Command::HubUpdate))
    {
        UpdateRoute::Hub(i)
    } else if installs.iter().any(|i| i.source == Source::Store) {
        UpdateRoute::Store
    } else if installs.is_empty() {
        UpdateRoute::NotInstalled
    } else {
        UpdateRoute::OwnApp
    }
}

/// The `update-progress` messages a hosted module gets for one line of a full (not check-only)
/// update run (HUB-APP-CONTRACT §5.2). `module_open`: the app is open in a Hub tab, so `done`
/// is followed by `restart-needed`.
pub fn module_msgs(line: &UpdateLine, module_open: bool) -> Vec<HubMsg> {
    let m = |state: ModState, progress: Option<f64>| HubMsg::UpdateProgress { state, progress };
    match line.state {
        UpdateState::Checking => vec![m(ModState::Checking, None)],
        UpdateState::Downloading => vec![m(ModState::Downloading, line.progress)],
        UpdateState::Installing => vec![m(ModState::Installing, line.progress)],
        UpdateState::Done if module_open => {
            vec![m(ModState::Done, None), m(ModState::RestartNeeded, None)]
        }
        UpdateState::Done => vec![m(ModState::Done, None)],
        UpdateState::Error => vec![m(ModState::Failed, None)],
        UpdateState::Available | UpdateState::None => Vec::new(),
    }
}

/// The initialization script for every WebView that hosts a module: the marker `embed.js`
/// looks for (D-H18). `id` is a catalog id (`a-z0-9-`), so it needs no escaping.
pub fn marker_script(id: &str) -> String {
    debug_assert!(crate::catalog::valid_id(id));
    format!(
        "Object.defineProperty(window,\"__FASTKIWI_HUB__\",{{value:Object.freeze({{proto:\"{}\",module:\"{id}\"}}),writable:false,configurable:false}});",
        crate::protocol::PROTO
    )
}
