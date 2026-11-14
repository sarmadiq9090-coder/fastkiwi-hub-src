//! Running an app under test: stdin and stdout piped, stderr discarded (as the Hub does), stdout
//! read line by line on a thread so every wait has a timeout.

use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::{Child, ChildStdin, Command, ExitStatus, Stdio};
use std::sync::mpsc::{Receiver, RecvTimeoutError, channel};
use std::time::{Duration, Instant};

pub struct Running {
    pub child: Child,
    stdin: Option<ChildStdin>,
    lines: Receiver<Vec<u8>>,
    exited: Option<ExitStatus>,
}

/// What [`Running::next_line`] got.
#[derive(Debug)]
pub enum Line {
    Text(Vec<u8>),
    /// stdout closed.
    Eof,
    Timeout,
}

impl Running {
    pub fn spawn(exe: &Path, args: &[&str]) -> std::io::Result<Running> {
        let mut child = Command::new(exe)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;
        let stdin = child.stdin.take();
        let stdout = child.stdout.take().expect("piped stdout");
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            let mut r = BufReader::new(stdout);
            loop {
                let mut buf = Vec::new();
                match r.read_until(b'\n', &mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {
                        if buf.ends_with(b"\n") {
                            buf.pop();
                        }
                        if buf.ends_with(b"\r") {
                            buf.pop();
                        }
                        if tx.send(buf).is_err() {
                            break;
                        }
                    }
                }
            }
        });
        Ok(Running {
            child,
            stdin,
            lines: rx,
            exited: None,
        })
    }

    pub fn pid(&self) -> u32 {
        self.child.id()
    }

    pub fn next_line(&self, timeout: Duration) -> Line {
        match self.lines.recv_timeout(timeout) {
            Ok(l) => Line::Text(l),
            Err(RecvTimeoutError::Timeout) => Line::Timeout,
            Err(RecvTimeoutError::Disconnected) => Line::Eof,
        }
    }

    /// Lines already printed and not yet read.
    pub fn pending_lines(&self) -> Vec<Vec<u8>> {
        self.lines.try_iter().collect()
    }

    pub fn close_stdin(&mut self) {
        self.stdin.take();
    }

    /// Wait up to `t` for the process to exit.
    pub fn wait_timeout(&mut self, t: Duration) -> Option<ExitStatus> {
        if self.exited.is_some() {
            return self.exited;
        }
        let end = Instant::now() + t;
        loop {
            if let Ok(Some(s)) = self.child.try_wait() {
                self.exited = Some(s);
                return self.exited;
            }
            if Instant::now() >= end {
                return None;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    pub fn kill(&mut self) {
        if self.exited.is_none() {
            let _ = self.child.kill();
            self.exited = self.child.wait().ok();
        }
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        self.kill();
    }
}
