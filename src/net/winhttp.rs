// Minimal blocking HTTP client built on Win32 WinHTTP.
//
// One `Client` owns a session handle and is thread-safe (WinHTTP sessions
// can be used from multiple threads per MSDN). Each `send()` call manages
// its own connection + request handle lifetime via RAII guards so failures
// at any point clean up correctly.
//
// We deliberately do NOT use `WinHttpCrackUrl` — the small `parse_url`
// helper below is enough for the HTTPS URLs this app actually talks to and
// keeps the call sites simpler.

use std::ffi::c_void;
use std::ptr::null_mut;

use serde::de::DeserializeOwned;
use serde::Serialize;
use windows::core::PCWSTR;
use windows::Win32::Networking::WinHttp::*;

use crate::os::string::to_utf16_nul;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("WinHTTP call failed: {context}")]
    Win { context: String },
    #[error("invalid URL: {0}")]
    Url(String),
    #[error("response was not valid UTF-8")]
    Utf8,
    #[error("JSON parse: {0}")]
    Json(#[from] serde_json::Error),
    #[error("response status {0}")]
    Status(u32),
}

pub struct Client {
    session: SessionHandle,
}

// WinHTTP session handles are safe to use concurrently per the Microsoft docs
// (https://learn.microsoft.com/en-us/windows/win32/winhttp/winhttp-functions).
unsafe impl Send for Client {}
unsafe impl Sync for Client {}

impl Client {
    /// Create a new HTTP client. `user_agent` is sent on every request.
    pub fn new(user_agent: &str) -> Result<Self, Error> {
        let ua = to_utf16_nul(user_agent);
        let session = unsafe {
            WinHttpOpen(
                PCWSTR::from_raw(ua.as_ptr()),
                WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
                PCWSTR::null(),
                PCWSTR::null(),
                0,
            )
        };
        if session.is_null() {
            return Err(Error::Win {
                context: "WinHttpOpen".into(),
            });
        }
        // Ask WinHTTP to decompress gzip/deflate transparently so callers
        // get plain bytes back from `Response::body()`. Best-effort; if it
        // fails the request still works, callers just see raw compressed
        // bytes on responses that opt-in to compression.
        unsafe {
            let flags: u32 = WINHTTP_DECOMPRESSION_FLAG_GZIP | WINHTTP_DECOMPRESSION_FLAG_DEFLATE;
            let flag_bytes = flags.to_ne_bytes();
            if let Err(e) = WinHttpSetOption(
                Some(session as *const c_void),
                WINHTTP_OPTION_DECOMPRESSION,
                Some(&flag_bytes),
            ) {
                log::warn!("WinHttpSetOption(DECOMPRESSION) failed: {e}");
            }
        }
        Ok(Self {
            session: SessionHandle(session),
        })
    }

    pub fn get<'a>(&'a self, url: &str) -> RequestBuilder<'a> {
        RequestBuilder::new(self, Method::Get, url)
    }

    pub fn post<'a>(&'a self, url: &str) -> RequestBuilder<'a> {
        RequestBuilder::new(self, Method::Post, url)
    }
}

#[derive(Clone, Copy)]
enum Method {
    Get,
    Post,
}

impl Method {
    fn verb(self) -> &'static str {
        match self {
            Method::Get => "GET",
            Method::Post => "POST",
        }
    }
}

pub struct RequestBuilder<'a> {
    client: &'a Client,
    method: Method,
    url: String,
    headers: Vec<(String, String)>,
    body: Option<Vec<u8>>,
    no_redirects: bool,
}

impl<'a> RequestBuilder<'a> {
    fn new(client: &'a Client, method: Method, url: &str) -> Self {
        Self {
            client,
            method,
            url: url.to_string(),
            headers: Vec::new(),
            body: None,
            no_redirects: false,
        }
    }

    pub fn no_redirects(mut self) -> Self {
        self.no_redirects = true;
        self
    }

    pub fn header(mut self, name: &str, value: &str) -> Self {
        self.headers.push((name.to_string(), value.to_string()));
        self
    }

    pub fn json_body<T: Serialize>(mut self, body: &T) -> Result<Self, Error> {
        self.body = Some(serde_json::to_vec(body)?);
        self.headers
            .push(("Content-Type".into(), "application/json".into()));
        Ok(self)
    }

    pub fn send(self) -> Result<Response, Error> {
        let parsed = parse_url(&self.url)?;
        let host_w = to_utf16_nul(&parsed.host);
        let path_w = to_utf16_nul(&parsed.path);
        let verb_w = to_utf16_nul(self.method.verb());

        let connect = unsafe {
            WinHttpConnect(
                self.client.session.0,
                PCWSTR::from_raw(host_w.as_ptr()),
                parsed.port,
                0,
            )
        };
        if connect.is_null() {
            return Err(Error::Win {
                context: "WinHttpConnect".into(),
            });
        }
        let _connect_guard = HandleGuard(connect);

        let flags = if parsed.secure {
            WINHTTP_FLAG_SECURE
        } else {
            WINHTTP_OPEN_REQUEST_FLAGS(0)
        };
        let request = unsafe {
            WinHttpOpenRequest(
                connect,
                PCWSTR::from_raw(verb_w.as_ptr()),
                PCWSTR::from_raw(path_w.as_ptr()),
                PCWSTR::null(),
                PCWSTR::null(),
                std::ptr::null::<PCWSTR>(),
                flags,
            )
        };
        if request.is_null() {
            return Err(Error::Win {
                context: "WinHttpOpenRequest".into(),
            });
        }
        let _request_guard = HandleGuard(request);
        if self.no_redirects {
            let policy: u32 = WINHTTP_OPTION_REDIRECT_POLICY_NEVER;
            unsafe {
                WinHttpSetOption(
                    Some(request),
                    WINHTTP_OPTION_REDIRECT_POLICY,
                    Some(&policy.to_ne_bytes()),
                )
            }
            .map_err(|_| Error::Win {
                context: "disable redirects".into(),
            })?;
        }

        // Combine headers into a single CRLF-separated string. The binding
        // takes a UTF-16 slice; length is derived from the slice and no
        // trailing NUL is required.
        if !self.headers.is_empty() {
            let header_str = self
                .headers
                .iter()
                .map(|(k, v)| format!("{k}: {v}"))
                .collect::<Vec<_>>()
                .join("\r\n");
            let header_w: Vec<u16> = header_str.encode_utf16().collect();
            unsafe {
                WinHttpAddRequestHeaders(
                    request,
                    &header_w[..],
                    WINHTTP_ADDREQ_FLAG_ADD | WINHTTP_ADDREQ_FLAG_REPLACE,
                )
            }
            .map_err(|e| Error::Win {
                context: format!("WinHttpAddRequestHeaders: {e}"),
            })?;
        }

        // Send body if present. dwTotalLength = body length; dwOptionalLength
        // mirrors it for synchronous sends with the buffer included up front.
        let body_bytes: &[u8] = self.body.as_deref().unwrap_or(&[]);
        let body_ptr = if body_bytes.is_empty() {
            None
        } else {
            Some(body_bytes.as_ptr() as *const c_void)
        };
        let body_len = body_bytes.len() as u32;
        unsafe { WinHttpSendRequest(request, None, body_ptr, body_len, body_len, 0) }.map_err(
            |e| Error::Win {
                context: format!("WinHttpSendRequest: {e}"),
            },
        )?;

        unsafe { WinHttpReceiveResponse(request, null_mut()) }.map_err(|e| Error::Win {
            context: format!("WinHttpReceiveResponse: {e}"),
        })?;

        let status = query_status_code(request)?;
        let headers = query_raw_headers(request)?;
        let body = read_body(request)?;

        Ok(Response {
            status,
            headers,
            body,
        })
    }
}

pub struct Response {
    status: u32,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl Response {
    pub fn status(&self) -> u32 {
        self.status
    }

    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    pub fn body(&self) -> &[u8] {
        &self.body
    }

    pub fn text(&self) -> Result<&str, Error> {
        std::str::from_utf8(&self.body).map_err(|_| Error::Utf8)
    }

    pub fn json<T: DeserializeOwned>(&self) -> Result<T, Error> {
        Ok(serde_json::from_slice(&self.body)?)
    }
}

// ---------- Low-level helpers ----------

struct SessionHandle(*mut c_void);

impl Drop for SessionHandle {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                let _ = WinHttpCloseHandle(self.0);
            }
        }
    }
}

struct HandleGuard(*mut c_void);

impl Drop for HandleGuard {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                let _ = WinHttpCloseHandle(self.0);
            }
        }
    }
}

fn query_status_code(request: *mut c_void) -> Result<u32, Error> {
    let mut status: u32 = 0;
    let mut size: u32 = std::mem::size_of::<u32>() as u32;
    unsafe {
        WinHttpQueryHeaders(
            request,
            WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
            PCWSTR::null(),
            Some((&mut status as *mut u32) as *mut c_void),
            &mut size,
            std::ptr::null_mut(),
        )
    }
    .map_err(|e| Error::Win {
        context: format!("WinHttpQueryHeaders(STATUS_CODE): {e}"),
    })?;
    Ok(status)
}

fn query_raw_headers(request: *mut c_void) -> Result<Vec<(String, String)>, Error> {
    // First call sizes the buffer (returns Err with ERROR_INSUFFICIENT_BUFFER
    // and writes the required byte count to `needed`).
    let mut needed: u32 = 0;
    let _ = unsafe {
        WinHttpQueryHeaders(
            request,
            WINHTTP_QUERY_RAW_HEADERS_CRLF,
            PCWSTR::null(),
            None,
            &mut needed,
            std::ptr::null_mut(),
        )
    };
    if needed == 0 {
        return Ok(Vec::new());
    }
    let chars = (needed as usize) / std::mem::size_of::<u16>();
    let mut buf: Vec<u16> = vec![0u16; chars];
    unsafe {
        WinHttpQueryHeaders(
            request,
            WINHTTP_QUERY_RAW_HEADERS_CRLF,
            PCWSTR::null(),
            Some(buf.as_mut_ptr() as *mut c_void),
            &mut needed,
            std::ptr::null_mut(),
        )
    }
    .map_err(|e| Error::Win {
        context: format!("WinHttpQueryHeaders(RAW_HEADERS_CRLF): {e}"),
    })?;
    let text = String::from_utf16_lossy(&buf[..chars.saturating_sub(1)]);
    Ok(parse_header_block(&text))
}

fn parse_header_block(block: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut lines = block.split("\r\n");
    let _ = lines.next(); // status line, e.g. "HTTP/1.1 200 OK"
    for line in lines {
        if line.is_empty() {
            continue;
        }
        if let Some((k, v)) = line.split_once(':') {
            out.push((k.trim().to_string(), v.trim().to_string()));
        }
    }
    out
}

fn read_body(request: *mut c_void) -> Result<Vec<u8>, Error> {
    let mut body = Vec::new();
    loop {
        let mut available: u32 = 0;
        unsafe { WinHttpQueryDataAvailable(request, &mut available) }.map_err(|e| Error::Win {
            context: format!("WinHttpQueryDataAvailable: {e}"),
        })?;
        if available == 0 {
            break;
        }
        let mut chunk = vec![0u8; available as usize];
        let mut read: u32 = 0;
        unsafe {
            WinHttpReadData(
                request,
                chunk.as_mut_ptr() as *mut c_void,
                available,
                &mut read,
            )
        }
        .map_err(|e| Error::Win {
            context: format!("WinHttpReadData: {e}"),
        })?;
        chunk.truncate(read as usize);
        body.append(&mut chunk);
    }
    Ok(body)
}

// ---------- URL parsing ----------

struct ParsedUrl {
    host: String,
    port: u16,
    path: String,
    secure: bool,
}

fn parse_url(url: &str) -> Result<ParsedUrl, Error> {
    let (scheme, rest) = url
        .split_once("://")
        .ok_or_else(|| Error::Url(url.to_string()))?;
    let secure = match scheme.to_ascii_lowercase().as_str() {
        "https" => true,
        "http" => false,
        other => return Err(Error::Url(format!("unsupported scheme: {other}"))),
    };
    let (host_port, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, "/"),
    };
    let (host, port) = match host_port.rsplit_once(':') {
        Some((h, p)) => (
            h.to_string(),
            p.parse::<u16>().map_err(|_| Error::Url(url.to_string()))?,
        ),
        None => (host_port.to_string(), if secure { 443 } else { 80 }),
    };
    if host.is_empty() {
        return Err(Error::Url(url.to_string()));
    }
    Ok(ParsedUrl {
        host,
        port,
        path: path.to_string(),
        secure,
    })
}
