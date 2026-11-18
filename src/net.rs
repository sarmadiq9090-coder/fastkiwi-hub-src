//! The Hub's only network code: one request with WinHTTP (the system's TLS and proxy
//! settings), to three fixed hosts. It reads small JSON files and icons, and asks a local
//! Ollama a question; it never downloads an installer or anything that runs.
//! `tests/guards.rs` keeps every other source free of network APIs and keeps the host list
//! here.

use windows::Win32::Networking::WinHttp::*;
use windows::core::{HSTRING, PCWSTR, w};

/// The two hosts the Hub talks to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Host {
    /// `api.github.com`: release metadata for update checks (public, no sign-in).
    GitHubApi,
    /// `fastkiwi.app`: the signed catalog (`/data/apps.json` and its `.minisig`) and icons.
    Site,
    /// Ollama on this PC only (`http://127.0.0.1:11434`), for «؟» questions in the bar.
    Ollama,
}

impl Host {
    fn name(self) -> &'static str {
        match self {
            Host::GitHubApi => "api.github.com",
            Host::Site => "fastkiwi.app",
            Host::Ollama => "127.0.0.1",
        }
    }
    fn port(self) -> u16 {
        match self {
            Host::Ollama => 11434,
            _ => INTERNET_DEFAULT_HTTPS_PORT,
        }
    }
    fn secure(self) -> bool {
        self != Host::Ollama
    }
}

#[derive(Debug)]
pub struct Resp {
    pub status: u16,
    pub etag: Option<String>,
    pub body: Vec<u8>,
}

struct Handle(*mut core::ffi::c_void);
impl Drop for Handle {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: a WinHTTP handle opened here and closed once.
            unsafe {
                let _ = WinHttpCloseHandle(self.0);
            }
        }
    }
}

fn open(p: *mut core::ffi::c_void, what: &str) -> Result<Handle, String> {
    if p.is_null() {
        Err(format!("{what} failed: {}", windows::core::Error::from_thread()))
    } else {
        Ok(Handle(p))
    }
}

/// GET `https://<host><path>`. `path` must start with `/` and hold no spaces or control
/// characters. At most `max` bytes are read; a longer body is an error. No redirects.
pub fn get(host: Host, path: &str, etag: Option<&str>, max: usize) -> Result<Resp, String> {
    request(host, path, etag, None, max, 10_000)
}

/// POST a JSON body (Ollama only). `timeout_ms` bounds the wait for the answer.
pub fn post_json(host: Host, path: &str, body: &str, max: usize, timeout_ms: i32) -> Result<Resp, String> {
    if host != Host::Ollama {
        return Err("POST is only for the local Ollama".into());
    }
    request(host, path, None, Some(body), max, timeout_ms)
}

fn request(
    host: Host,
    path: &str,
    etag: Option<&str>,
    body: Option<&str>,
    max: usize,
    receive_ms: i32,
) -> Result<Resp, String> {
    if !path.starts_with('/')
        || path.len() > 512
        || path.chars().any(|c| c.is_control() || c.is_whitespace())
    {
        return Err("bad path".into());
    }
    let ua = HSTRING::from(format!(
        "FastKiwi-Hub/{} (+https://fastkiwi.app/)",
        env!("CARGO_PKG_VERSION")
    ));
    // SAFETY: WinHTTP calls on handles owned by `Handle`, with strings that outlive the calls.
    unsafe {
        let session = open(
            WinHttpOpen(
                &ua,
                WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
                PCWSTR::null(),
                PCWSTR::null(),
                0,
            ),
            "WinHttpOpen",
        )?;
        let connect_ms = if host == Host::Ollama { 1500 } else { 8000 };
        let _ = WinHttpSetTimeouts(session.0, connect_ms, connect_ms, 8000, receive_ms);
        let protocols = (WINHTTP_FLAG_SECURE_PROTOCOL_TLS1_2 | WINHTTP_FLAG_SECURE_PROTOCOL_TLS1_3)
            .to_ne_bytes();
        // TLS 1.3 is unknown to older Windows 10 builds: then TLS 1.2 only.
        if WinHttpSetOption(
            Some(session.0),
            WINHTTP_OPTION_SECURE_PROTOCOLS,
            Some(&protocols),
        )
        .is_err()
        {
            let _ = WinHttpSetOption(
                Some(session.0),
                WINHTTP_OPTION_SECURE_PROTOCOLS,
                Some(&WINHTTP_FLAG_SECURE_PROTOCOL_TLS1_2.to_ne_bytes()),
            );
        }
        let connect = open(
            WinHttpConnect(
                session.0,
                &HSTRING::from(host.name()),
                host.port(),
                0,
            ),
            "WinHttpConnect",
        )?;
        let request = open(
            WinHttpOpenRequest(
                connect.0,
                if body.is_some() { w!("POST") } else { w!("GET") },
                &HSTRING::from(path),
                PCWSTR::null(),
                PCWSTR::null(),
                std::ptr::null(),
                if host.secure() {
                    WINHTTP_FLAG_SECURE
                } else {
                    WINHTTP_OPEN_REQUEST_FLAGS(0)
                },
            ),
            "WinHttpOpenRequest",
        )?;
        let never = WINHTTP_OPTION_REDIRECT_POLICY_NEVER.to_ne_bytes();
        let _ = WinHttpSetOption(
            Some(request.0),
            WINHTTP_OPTION_REDIRECT_POLICY,
            Some(&never),
        );
        let mut headers = String::new();
        if host == Host::GitHubApi {
            headers.push_str("Accept: application/vnd.github+json\r\nX-GitHub-Api-Version: 2022-11-28\r\n");
        }
        if body.is_some() {
            headers.push_str("Content-Type: application/json\r\n");
        }
        if let Some(e) = etag.filter(|e| e.len() < 200 && !e.contains(['\r', '\n'])) {
            headers.push_str(&format!("If-None-Match: {e}\r\n"));
        }
        let wide: Vec<u16> = headers.encode_utf16().collect();
        let payload = body.map(str::as_bytes).unwrap_or(&[]);
        WinHttpSendRequest(
            request.0,
            (!wide.is_empty()).then_some(&wide[..]),
            (!payload.is_empty()).then(|| payload.as_ptr().cast()),
            payload.len() as u32,
            payload.len() as u32,
            0,
        )
        .map_err(|e| format!("send: {e}"))?;
        WinHttpReceiveResponse(request.0, std::ptr::null_mut()).map_err(|e| format!("receive: {e}"))?;

        let mut status: u32 = 0;
        let mut len = 4u32;
        WinHttpQueryHeaders(
            request.0,
            WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
            PCWSTR::null(),
            Some(&mut status as *mut u32 as *mut _),
            &mut len,
            std::ptr::null_mut(),
        )
        .map_err(|e| format!("status: {e}"))?;

        let mut buf = [0u16; 256];
        let mut blen = (buf.len() * 2) as u32;
        let etag = WinHttpQueryHeaders(
            request.0,
            WINHTTP_QUERY_ETAG,
            PCWSTR::null(),
            Some(buf.as_mut_ptr().cast()),
            &mut blen,
            std::ptr::null_mut(),
        )
        .ok()
        .map(|_| String::from_utf16_lossy(&buf[..(blen as usize / 2).min(buf.len())]));

        let mut body = Vec::new();
        loop {
            let mut avail = 0u32;
            WinHttpQueryDataAvailable(request.0, &mut avail).map_err(|e| format!("read: {e}"))?;
            if avail == 0 {
                break;
            }
            if body.len() + avail as usize > max {
                return Err("answer too large".into());
            }
            let start = body.len();
            body.resize(start + avail as usize, 0);
            let mut got = 0u32;
            WinHttpReadData(
                request.0,
                body[start..].as_mut_ptr().cast(),
                avail,
                &mut got,
            )
            .map_err(|e| format!("read: {e}"))?;
            body.truncate(start + got as usize);
            if got == 0 {
                break;
            }
        }
        Ok(Resp {
            status: status as u16,
            etag,
            body,
        })
    }
}
