//! A minimal HTTP/1.1 client for 127.0.0.1, enough to probe an app's hub-host guards. Every
//! request is sent exactly as written (the caller chooses the Host, Origin and Cookie headers,
//! or leaves them out), with `Connection: close`.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::Duration;

const TIMEOUT: Duration = Duration::from_secs(5);
const MAX_RESPONSE: usize = 8 * 1024 * 1024;

#[derive(Debug, Clone)]
pub struct Resp {
    pub status: u16,
    /// Header names lowercased, values trimmed, in order.
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Resp {
    pub fn header(&self, name: &str) -> Option<&str> {
        let name = name.to_ascii_lowercase();
        self.headers
            .iter()
            .find(|(n, _)| *n == name)
            .map(|(_, v)| v.as_str())
    }
    pub fn all(&self, name: &str) -> Vec<&str> {
        let name = name.to_ascii_lowercase();
        self.headers
            .iter()
            .filter(|(n, _)| *n == name)
            .map(|(_, v)| v.as_str())
            .collect()
    }
}

/// Send one request to 127.0.0.1:`port`. `headers` are written verbatim, in order.
pub fn request(
    port: u16,
    method: &str,
    target: &str,
    headers: &[(&str, &str)],
) -> std::io::Result<Resp> {
    request_to(
        SocketAddr::from(([127, 0, 0, 1], port)),
        method,
        target,
        headers,
    )
}

pub fn request_to(
    addr: SocketAddr,
    method: &str,
    target: &str,
    headers: &[(&str, &str)],
) -> std::io::Result<Resp> {
    let mut s = TcpStream::connect_timeout(&addr, TIMEOUT)?;
    s.set_read_timeout(Some(TIMEOUT))?;
    s.set_write_timeout(Some(TIMEOUT))?;
    let mut head = format!("{method} {target} HTTP/1.1\r\n");
    for (n, v) in headers {
        head.push_str(&format!("{n}: {v}\r\n"));
    }
    if !matches!(method, "GET" | "HEAD" | "OPTIONS") {
        head.push_str("Content-Length: 0\r\n");
    }
    head.push_str("Connection: close\r\n\r\n");
    s.write_all(head.as_bytes())?;
    let mut raw = Vec::new();
    (&mut s).take(MAX_RESPONSE as u64).read_to_end(&mut raw)?;
    parse(&raw, method == "HEAD")
}

fn bad(m: &str) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidData, m.to_string())
}

pub fn parse(raw: &[u8], head_only: bool) -> std::io::Result<Resp> {
    let end = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .ok_or_else(|| bad("no end of headers"))?;
    let head = std::str::from_utf8(&raw[..end]).map_err(|_| bad("headers not UTF-8"))?;
    let mut lines = head.split("\r\n");
    let status_line = lines.next().ok_or_else(|| bad("empty response"))?;
    let mut parts = status_line.splitn(3, ' ');
    let version = parts.next().unwrap_or("");
    if !version.starts_with("HTTP/1.") {
        return Err(bad("not HTTP/1.x"));
    }
    let status: u16 = parts
        .next()
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| bad("bad status"))?;
    let mut headers = Vec::new();
    for l in lines {
        let (n, v) = l.split_once(':').ok_or_else(|| bad("bad header line"))?;
        headers.push((n.trim().to_ascii_lowercase(), v.trim().to_string()));
    }
    let rest = &raw[end + 4..];
    let mut resp = Resp {
        status,
        headers,
        body: Vec::new(),
    };
    if head_only || status == 204 || status == 304 || (100..200).contains(&status) {
        return Ok(resp);
    }
    let chunked = resp
        .header("transfer-encoding")
        .is_some_and(|v| v.to_ascii_lowercase().contains("chunked"));
    resp.body = if chunked {
        dechunk(rest)?
    } else if let Some(len) = resp
        .header("content-length")
        .and_then(|v| v.parse::<usize>().ok())
    {
        rest.get(..len).ok_or_else(|| bad("short body"))?.to_vec()
    } else {
        rest.to_vec()
    };
    Ok(resp)
}

fn dechunk(mut b: &[u8]) -> std::io::Result<Vec<u8>> {
    let mut out = Vec::new();
    loop {
        let nl = b
            .windows(2)
            .position(|w| w == b"\r\n")
            .ok_or_else(|| bad("bad chunk"))?;
        let size_txt = std::str::from_utf8(&b[..nl]).map_err(|_| bad("bad chunk size"))?;
        let size = usize::from_str_radix(size_txt.split(';').next().unwrap_or("").trim(), 16)
            .map_err(|_| bad("bad chunk size"))?;
        b = &b[nl + 2..];
        if size == 0 {
            return Ok(out);
        }
        out.extend_from_slice(b.get(..size).ok_or_else(|| bad("short chunk"))?);
        b = b.get(size + 2..).ok_or_else(|| bad("short chunk"))?;
    }
}
