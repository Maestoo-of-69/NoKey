//! Tiny HTTPS client on top of WinHTTP (built into Windows). Uses the system
//! proxy settings and certificate store; no TLS library is bundled.

use crate::win::{last_error, wide};
use std::ffi::c_void;
use std::ptr::{null, null_mut};
use windows_sys::Win32::Networking::WinHttp::*;

const TIMEOUT_RESOLVE_MS: i32 = 10_000;
const TIMEOUT_CONNECT_MS: i32 = 10_000;
const TIMEOUT_SEND_MS: i32 = 15_000;
const TIMEOUT_RECEIVE_MS: i32 = 30_000;
const MAX_BODY: usize = 64 * 1024 * 1024;

pub struct Response {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Response {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }
}

struct Handle(*mut c_void);

impl Drop for Handle {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { WinHttpCloseHandle(self.0) };
        }
    }
}

/// Splits `https://host[:port]/path` into parts. Only HTTPS is allowed.
pub fn parse_url(url: &str) -> Result<(String, u16, String), String> {
    let rest = url.strip_prefix("https://").ok_or_else(|| format!("Only https:// URLs are supported: {url}"))?;
    let (authority, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, "/"),
    };
    let (host, port) = match authority.rsplit_once(':') {
        Some((h, p)) => (h, p.parse::<u16>().map_err(|_| format!("Bad port in {url}"))?),
        None => (authority, 443),
    };
    if host.is_empty() {
        return Err(format!("Bad URL: {url}"));
    }
    Ok((host.to_string(), port, path.to_string()))
}

fn net_error(context: &str) -> String {
    let code = last_error();
    let why = match code {
        12002 => "the request timed out",
        12007 => "the server name could not be resolved (no internet?)",
        12029 | 12030 => "could not connect to the server",
        12175 | 12157 => "the secure connection failed",
        12152 => "the server sent an invalid response",
        _ => "network error",
    };
    format!("Network: {context}: {why} (code {code})")
}

pub type Sink<'a> = &'a mut dyn FnMut(&[u8]) -> Result<(), String>;

/// Performs one HTTPS request. `sink` receives the body in chunks; when it
/// is `None` the body is collected into `Response::body`.
pub fn request_with(
    method: &str,
    url: &str,
    headers: &[(&str, &str)],
    body: Option<&[u8]>,
    mut sink: Option<Sink>,
) -> Result<Response, String> {
    let (host, port, path) = parse_url(url)?;
    let agent = wide("NoKey");
    unsafe {
        let mut session = Handle(WinHttpOpen(agent.as_ptr(), WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, null(), null(), 0));
        if session.0.is_null() {
            session = Handle(WinHttpOpen(agent.as_ptr(), WINHTTP_ACCESS_TYPE_DEFAULT_PROXY, null(), null(), 0));
        }
        if session.0.is_null() {
            return Err(net_error("open"));
        }
        WinHttpSetTimeouts(session.0, TIMEOUT_RESOLVE_MS, TIMEOUT_CONNECT_MS, TIMEOUT_SEND_MS, TIMEOUT_RECEIVE_MS);
        let host_w = wide(&host);
        let connect = Handle(WinHttpConnect(session.0, host_w.as_ptr(), port, 0));
        if connect.0.is_null() {
            return Err(net_error("connect"));
        }
        let method_w = wide(method);
        let path_w = wide(&path);
        let req = Handle(WinHttpOpenRequest(
            connect.0,
            method_w.as_ptr(),
            path_w.as_ptr(),
            null(),
            null(),
            null(),
            WINHTTP_FLAG_SECURE,
        ));
        if req.0.is_null() {
            return Err(net_error("open request"));
        }
        let header_text: String = headers.iter().map(|(k, v)| format!("{k}: {v}\r\n")).collect();
        let header_w: Vec<u16> = header_text.encode_utf16().collect();
        let (body_ptr, body_len) = match body {
            Some(b) => (b.as_ptr() as *const c_void, b.len() as u32),
            None => (null(), 0),
        };
        let header_ptr = if header_w.is_empty() { null() } else { header_w.as_ptr() };
        if WinHttpSendRequest(req.0, header_ptr, header_w.len() as u32, body_ptr, body_len, body_len, 0) == 0 {
            return Err(net_error("send"));
        }
        if WinHttpReceiveResponse(req.0, null_mut()) == 0 {
            return Err(net_error("receive"));
        }

        let mut status: u32 = 0;
        let mut size = 4u32;
        WinHttpQueryHeaders(
            req.0,
            WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
            null(),
            &mut status as *mut u32 as *mut c_void,
            &mut size,
            null_mut(),
        );
        let headers = read_headers(req.0);

        let mut collected = Vec::new();
        let mut total = 0usize;
        loop {
            let mut available = 0u32;
            if WinHttpQueryDataAvailable(req.0, &mut available) == 0 {
                return Err(net_error("read"));
            }
            if available == 0 {
                break;
            }
            let mut chunk = vec![0u8; available as usize];
            let mut read = 0u32;
            if WinHttpReadData(req.0, chunk.as_mut_ptr() as *mut c_void, available, &mut read) == 0 {
                return Err(net_error("read"));
            }
            chunk.truncate(read as usize);
            total += chunk.len();
            if total > MAX_BODY {
                return Err("Network: response is too large".into());
            }
            match sink.as_mut() {
                Some(s) => s(&chunk)?,
                None => collected.extend_from_slice(&chunk),
            }
        }
        Ok(Response { status: status as u16, headers, body: collected })
    }
}

unsafe fn read_headers(req: *mut c_void) -> Vec<(String, String)> {
    let mut size = 0u32;
    unsafe {
        WinHttpQueryHeaders(req, WINHTTP_QUERY_RAW_HEADERS_CRLF, null(), null_mut(), &mut size, null_mut());
        if size == 0 {
            return Vec::new();
        }
        let mut buf = vec![0u16; size as usize / 2 + 1];
        if WinHttpQueryHeaders(
            req,
            WINHTTP_QUERY_RAW_HEADERS_CRLF,
            null(),
            buf.as_mut_ptr() as *mut c_void,
            &mut size,
            null_mut(),
        ) == 0
        {
            return Vec::new();
        }
        buf.truncate(size as usize / 2);
        String::from_utf16_lossy(&buf)
            .lines()
            .skip(1)
            .filter_map(|l| l.split_once(':').map(|(k, v)| (k.trim().to_string(), v.trim().to_string())))
            .collect()
    }
}

pub fn request(method: &str, url: &str, headers: &[(&str, &str)], body: Option<&[u8]>) -> Result<Response, String> {
    request_with(method, url, headers, body, None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_urls() {
        assert_eq!(
            parse_url("https://discord.com/api/v10/users/@me").unwrap(),
            ("discord.com".into(), 443, "/api/v10/users/@me".into())
        );
        assert_eq!(parse_url("https://localhost:8443").unwrap(), ("localhost".into(), 8443, "/".into()));
        assert!(parse_url("http://discord.com/").is_err());
        assert!(parse_url("https:///x").is_err());
    }
}
