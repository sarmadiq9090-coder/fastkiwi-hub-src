//! Running an app's hub commands (fastkiwi-hub-cli@1, docs/HUB-APP-CONTRACT.md §4 and §5).
//!
//! This is the only file in the Hub that starts a process with pipes. It starts only the exe
//! that detection found, with only the flags of the contract, and only after the copy's
//! `fastkiwi-hub.json` declared that command. Standalone launches stay ShellExecute and forget
//! (`win::launch`).
//!
//! Every child gets stdin and stdout as pipes and stderr as `NUL`, and `CREATE_NO_WINDOW`, so
//! a console app never flashes a console. Each line it prints is validated with `hubcli`;
//! anything outside the contract ends the run.
//!
//! The Hub owns only the processes it starts here. No Job object; when the Hub exits, its pipe
//! ends close, and a conforming app exits on stdin EOF.

use hubcli::{Handshake, UpdateLine, UpdateRun, UpdateState};
use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{Receiver, RecvTimeoutError, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// After the Hub closes a hub-host's stdin, it terminates the process if it is still running
/// this long after (contract §4.2).
pub const STOP_GRACE: Duration = Duration::from_secs(5);

fn command(exe: &Path, args: &[&str]) -> Command {
    let mut c = Command::new(exe);
    c.args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    if let Some(dir) = exe.parent() {
        c.current_dir(dir);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        c.creation_flags(CREATE_NO_WINDOW);
    }
    c
}

/// Read stdout line by line on a thread (lines longer than the contract allows are cut, so a
/// broken app cannot make the Hub buffer without end; they then fail validation).
fn lines(stdout: std::process::ChildStdout) -> Receiver<String> {
    let (tx, rx) = channel();
    std::thread::spawn(move || {
        let mut r = BufReader::new(stdout);
        loop {
            let mut buf = Vec::new();
            match (&mut r)
                .take(hubcli::MAX_LINE as u64 * 2)
                .read_until(b'\n', &mut buf)
            {
                Ok(0) | Err(_) => break,
                Ok(_) => {
                    // A cut line: skip the rest of it, a bounded piece at a time.
                    while !buf.ends_with(b"\n") {
                        let mut rest = Vec::new();
                        match (&mut r).take(4096).read_until(b'\n', &mut rest) {
                            Ok(0) | Err(_) => break,
                            Ok(_) if rest.ends_with(b"\n") => break,
                            Ok(_) => {}
                        }
                    }
                    let text = String::from_utf8(buf).unwrap_or_default();
                    if tx.send(text).is_err() {
                        break;
                    }
                }
            }
        }
    });
    rx
}

use std::io::Read as _;

/// Wait for `child` to exit (polling, so others can still kill it). `None` after `limit`.
fn wait(child: &Mutex<Child>, limit: Duration) -> Option<Option<i32>> {
    let end = Instant::now() + limit;
    loop {
        if let Ok(mut c) = child.lock()
            && let Ok(Some(s)) = c.try_wait()
        {
            return Some(s.code());
        }
        if Instant::now() >= end {
            return None;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn kill(child: &Mutex<Child>) {
    if let Ok(mut c) = child.lock() {
        let _ = c.kill();
        let _ = c.wait();
    }
}

// ---------------------------------------------------------------------------------------------
// --hub-host

#[derive(Debug, Clone, PartialEq)]
pub enum HostEvent {
    /// A valid handshake: the page is at `handshake.enter_url()`.
    Ready(Handshake),
    /// No valid handshake. The process has been terminated.
    Failed(HostFailure),
    /// The process ended (after `Ready`, or after the Hub stopped it).
    Exited(Option<i32>),
}

#[derive(Debug, Clone, PartialEq)]
pub enum HostFailure {
    /// stdout closed (the app exited, or does not know `--hub-host` after all).
    NoHandshake,
    TimedOut,
    /// A line that is not a valid handshake for this app (the reason, for the dev log only).
    BadHandshake(String),
}

/// A running `<exe> --hub-host`.
pub struct HostProcess {
    child: Arc<Mutex<Child>>,
    stdin: Option<ChildStdin>,
    pid: u32,
}

impl HostProcess {
    /// Start `exe --hub-host` for app `app`. `on` is called from a worker thread: once with
    /// `Ready` or `Failed`, then (after `Ready`) once with `Exited`.
    pub fn start(
        exe: &Path,
        app: &str,
        handshake_timeout: Duration,
        on: impl Fn(HostEvent) + Send + 'static,
    ) -> std::io::Result<HostProcess> {
        let mut child = command(exe, &[hubcli::Command::HubHost.flag()]).spawn()?;
        let stdin = child.stdin.take();
        let stdout = child.stdout.take().expect("piped stdout");
        let pid = child.id();
        let child = Arc::new(Mutex::new(child));
        let rx = lines(stdout);
        let app = app.to_string();
        let watch = child.clone();
        std::thread::spawn(move || {
            let first = match rx.recv_timeout(handshake_timeout) {
                Ok(l) => hubcli::parse_handshake(&l, &app)
                    .map_err(|e| HostFailure::BadHandshake(e.to_string())),
                Err(RecvTimeoutError::Timeout) => Err(HostFailure::TimedOut),
                Err(RecvTimeoutError::Disconnected) => Err(HostFailure::NoHandshake),
            };
            match first {
                Ok(h) => on(HostEvent::Ready(h)),
                Err(f) => {
                    kill(&watch);
                    on(HostEvent::Failed(f));
                    return;
                }
            }
            // Anything more on stdout breaks the contract but is harmless: drop it.
            while rx.recv().is_ok() {}
            let code = loop {
                if let Some(c) = wait(&watch, Duration::from_secs(3600)) {
                    break c;
                }
            };
            on(HostEvent::Exited(code));
        });
        Ok(HostProcess { child, stdin, pid })
    }

    pub fn pid(&self) -> u32 {
        self.pid
    }

    /// Close stdin (the app should exit within 2 s) and terminate it if it still runs after
    /// `grace`. Returns at once.
    pub fn stop_with(mut self, grace: Duration) {
        self.stop_inner(grace);
    }

    fn stop_inner(&mut self, grace: Duration) {
        if self.stdin.take().is_none() {
            return;
        }
        let child = self.child.clone();
        std::thread::spawn(move || {
            if wait(&child, grace).is_none() {
                kill(&child);
            }
        });
    }
}

impl Drop for HostProcess {
    fn drop(&mut self) {
        self.stop_inner(STOP_GRACE);
    }
}

// ---------------------------------------------------------------------------------------------
// --hub-update

#[derive(Debug, Clone, PartialEq)]
pub enum UpdateEvent {
    /// One valid line, in a valid order.
    Line(UpdateLine),
    /// The run is over.
    End(UpdateEnd),
}

#[derive(Debug, Clone, PartialEq)]
pub enum UpdateEnd {
    /// The final line (`available`, `done`, `none` or `error`) and a matching exit code.
    Final(UpdateLine),
    /// The app broke the contract (bad line, order, silence, exit code): the Hub shows
    /// `error` `internal`. The reason is for the dev log only.
    Broken(String),
}

impl UpdateEnd {
    /// What the Hub shows: the final line, or `error internal` for a broken run.
    pub fn shown(&self) -> UpdateLine {
        match self {
            UpdateEnd::Final(l) => l.clone(),
            UpdateEnd::Broken(_) => UpdateLine {
                code: Some(hubcli::UpdateCode::Internal),
                ..UpdateLine::new(UpdateState::Error)
            },
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct UpdateTiming {
    pub first_line: Duration,
    pub silence: Duration,
}

impl Default for UpdateTiming {
    fn default() -> Self {
        UpdateTiming {
            first_line: hubcli::UPDATE_FIRST_LINE,
            silence: hubcli::UPDATE_SILENCE,
        }
    }
}

/// A running `--hub-update`. Dropping it does not cancel; call [`UpdateHandle::cancel`].
pub struct UpdateHandle {
    stdin: Arc<Mutex<Option<ChildStdin>>>,
}

impl UpdateHandle {
    /// Close stdin: the app cancels if it has not reached `installing` (contract §5.2).
    pub fn cancel(&self) {
        if let Ok(mut s) = self.stdin.lock() {
            s.take();
        }
    }
}

/// The arguments the Hub passes: `--hub-update [--check-only] --hub-owner <hwnd>`.
pub fn update_args(check_only: bool, owner: isize) -> Vec<String> {
    let mut a = vec![hubcli::Command::HubUpdate.flag().to_string()];
    if check_only {
        a.push("--check-only".into());
    }
    a.push("--hub-owner".into());
    a.push(owner.max(0).to_string());
    a
}

/// Start `exe --hub-update …`. `on` is called from a worker thread for each valid line and
/// once with `End`.
pub fn start_update(
    exe: &Path,
    check_only: bool,
    owner: isize,
    timing: UpdateTiming,
    on: impl Fn(UpdateEvent) + Send + 'static,
) -> std::io::Result<UpdateHandle> {
    let args = update_args(check_only, owner);
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let mut child = command(exe, &args).spawn()?;
    let stdin = Arc::new(Mutex::new(child.stdin.take()));
    let stdout = child.stdout.take().expect("piped stdout");
    let child = Arc::new(Mutex::new(child));
    let rx = lines(stdout);
    let keep = stdin.clone();
    std::thread::spawn(move || {
        let _stdin = keep; // held open until the run ends, unless cancelled
        let mut run = UpdateRun::new(check_only);
        let mut last: Option<UpdateState> = None;
        let mut wait_for = timing.first_line;
        // A broken run: stop the process unless it is installing (never cut an install).
        let broken = |why: String, last: Option<UpdateState>| {
            if last != Some(UpdateState::Installing) {
                kill(&child);
            }
            on(UpdateEvent::End(UpdateEnd::Broken(why)));
        };
        loop {
            match rx.recv_timeout(wait_for) {
                Ok(text) => {
                    let line = match hubcli::parse_update_line(&text) {
                        Ok(l) => l,
                        Err(e) => return broken(e.to_string(), last),
                    };
                    if let Err(e) = run.push(&line) {
                        return broken(e.to_string(), last);
                    }
                    last = Some(line.state);
                    on(UpdateEvent::Line(line));
                    wait_for = timing.silence;
                }
                Err(RecvTimeoutError::Timeout) => {
                    return broken(format!("silent for {} s", wait_for.as_secs()), last);
                }
                Err(RecvTimeoutError::Disconnected) => break,
            }
        }
        let code = wait(&child, Duration::from_secs(10));
        let Some(code) = code else {
            return broken("did not exit after its last line".into(), last);
        };
        match run.finish(code) {
            Ok(l) => on(UpdateEvent::End(UpdateEnd::Final(l.clone()))),
            Err(e) => on(UpdateEvent::End(UpdateEnd::Broken(e.to_string()))),
        }
    });
    Ok(UpdateHandle { stdin })
}
