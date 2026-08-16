//! Ein kleiner HTTP-Absender auf WinHTTP.
//!
//! No HTTP crate: WinHTTP ships with Windows, uses the system certificate
//! store and the system proxy settings, and costs nothing in binary size. The
//! promise this program makes — one `.exe`, no runtime to install — is easier
//! to keep with the operating system's own client than with a TLS stack of our
//! own.
//!
//! What it does: one request, body held in memory, answer read whole. That is
//! the shape of the job (a picture goes out, a smaller picture comes back),
//! and streaming would buy nothing but ways to go wrong.
//!
//! Every handle here is a bare `*mut c_void`. WinHTTP has no handle type in
//! the bindings and therefore no `Drop`, so [`Handle`] does that job; without
//! it every early return would leak a session.

use std::ffi::c_void;

use anyhow::{Context as _, Result, bail};
use windows::Win32::Networking::WinHttp::{
    INTERNET_DEFAULT_HTTPS_PORT, URL_COMPONENTS, WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
    WINHTTP_ADDREQ_FLAG_ADD, WINHTTP_FLAG_SECURE, WINHTTP_OPEN_REQUEST_FLAGS,
    WINHTTP_QUERY_FLAG_NUMBER, WINHTTP_QUERY_RAW_HEADERS_CRLF, WINHTTP_QUERY_STATUS_CODE,
    WinHttpAddRequestHeaders, WinHttpCloseHandle, WinHttpConnect, WinHttpCrackUrl, WinHttpOpen,
    WinHttpOpenRequest, WinHttpQueryHeaders, WinHttpReadData, WinHttpReceiveResponse,
    WinHttpSendRequest, WinHttpSetTimeouts,
};
use windows::core::PCWSTR;

use crate::favourites::Header;

/// What was sent.
pub struct Request {
    pub body: Vec<u8>,
    pub content_type: String,
}

/// What came back.
pub struct Answer {
    pub status: u32,
    pub body: Vec<u8>,
    /// Every response header, as sent, one per entry.
    pub headers: Vec<(String, String)>,
}

impl Answer {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }

    pub fn is_ok(&self) -> bool {
        (200..300).contains(&self.status)
    }
}

impl Request {
    /// The file as the entire body. What several APIs expect — TinyPNG among
    /// them, which takes the image itself and no wrapping at all.
    pub fn raw(bytes: Vec<u8>, mime: &str) -> Self {
        Request {
            body: bytes,
            content_type: mime.to_string(),
        }
    }

    /// `multipart/form-data`: what an upload form sends, and therefore what
    /// most self-hosted tools accept.
    pub fn multipart(
        field: &str,
        file_name: &str,
        bytes: Vec<u8>,
        mime: &str,
        extra: &[Header],
    ) -> Self {
        let boundary = boundary();
        let mut body = Vec::with_capacity(bytes.len() + 512);

        for pair in extra {
            body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
            body.extend_from_slice(
                format!(
                    "Content-Disposition: form-data; name=\"{}\"\r\n\r\n{}\r\n",
                    escape(&pair.name),
                    pair.value
                )
                .as_bytes(),
            );
        }

        body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
        body.extend_from_slice(
            format!(
                "Content-Disposition: form-data; name=\"{}\"; filename=\"{}\"\r\n\
                 Content-Type: {mime}\r\n\r\n",
                escape(field),
                escape(file_name)
            )
            .as_bytes(),
        );
        body.extend_from_slice(&bytes);
        body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());

        Request {
            body,
            content_type: format!("multipart/form-data; boundary={boundary}"),
        }
    }
}

/// A separator that cannot occur inside the payload by accident.
///
/// Time plus process id rather than a random number: this program carries no
/// random source, and two uploads from the same process in the same
/// nanosecond do not happen.
fn boundary() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("----ctxmenu{:x}{:x}", std::process::id(), nanos)
}

/// Quotes for a `Content-Disposition` value.
///
/// A file called `Bild".png` would otherwise close the quoted string early and
/// let the rest of the name be read as parameters.
fn escape(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

/// Sends one request and reads the whole answer.
pub fn send(url: &str, method: &str, headers: &[Header], request: Request) -> Result<Answer> {
    let target = Url::parse(url)?;

    // 15 s to connect, 60 s to send, 120 s to receive: an upload of a few
    // megabytes over a slow line is normal, a wait of minutes is not. Without
    // this WinHTTP waits 30 s per stage and never gives up on receive.
    let session = Handle::session()?;
    unsafe { WinHttpSetTimeouts(session.0, 10_000, 15_000, 60_000, 120_000) }
        .context("WinHttpSetTimeouts")?;

    let connect = Handle::connect(&session, &target)?;
    let request_handle = Handle::request(&connect, &target, method)?;

    let mut lines = format!("Content-Type: {}\r\n", request.content_type);
    for header in headers {
        lines.push_str(&format!("{}: {}\r\n", header.name, header.value));
    }
    let wide_headers: Vec<u16> = lines.encode_utf16().collect();
    unsafe { WinHttpAddRequestHeaders(request_handle.0, &wide_headers, WINHTTP_ADDREQ_FLAG_ADD) }
        .context("WinHttpAddRequestHeaders")?;

    let length = u32::try_from(request.body.len()).context(
        "\x1eDatei zu groß für eine einzelne Anfrage\x1ffile too large for one request\x1d",
    )?;

    unsafe {
        WinHttpSendRequest(
            request_handle.0,
            None,
            Some(request.body.as_ptr() as *const c_void),
            length,
            length,
            0,
        )
    }
    .context("WinHttpSendRequest")?;

    unsafe { WinHttpReceiveResponse(request_handle.0, std::ptr::null_mut()) }
        .context("WinHttpReceiveResponse")?;

    let answer = Answer {
        status: status_of(&request_handle)?,
        headers: headers_of(&request_handle)?,
        body: body_of(&request_handle)?,
    };

    if !answer.is_ok() {
        // The body of an error answer is usually the only thing that says what
        // went wrong, so it goes into the message rather than the log.
        let hint = String::from_utf8_lossy(&answer.body);
        let hint = hint.trim();
        bail!(
            "\x1eDer Dienst antwortete mit {}\x1fthe service answered {}\x1d{}",
            answer.status,
            answer.status,
            if hint.is_empty() {
                String::new()
            } else {
                format!(": {}", &hint[..hint.len().min(400)])
            }
        );
    }

    Ok(answer)
}

/// Fetches a document, with whatever headers it needs to be allowed to.
///
/// Separate from `download` because a service description often sits behind the
/// same key as the service itself, and separate from `send` because a GET with
/// an empty `Content-Type` line is a request some servers answer with 400.
pub fn fetch(url: &str, headers: &[Header]) -> Result<Vec<u8>> {
    let target = Url::parse(url)?;

    let session = Handle::session()?;
    unsafe { WinHttpSetTimeouts(session.0, 10_000, 15_000, 60_000, 120_000) }
        .context("WinHttpSetTimeouts")?;
    let connect = Handle::connect(&session, &target)?;
    let request = Handle::request(&connect, &target, "GET")?;

    if !headers.is_empty() {
        let mut lines = String::new();
        for header in headers {
            lines.push_str(&format!("{}: {}\r\n", header.name, header.value));
        }
        let wide: Vec<u16> = lines.encode_utf16().collect();
        unsafe { WinHttpAddRequestHeaders(request.0, &wide, WINHTTP_ADDREQ_FLAG_ADD) }
            .context("WinHttpAddRequestHeaders")?;
    }

    unsafe { WinHttpSendRequest(request.0, None, None, 0, 0, 0) }.context("WinHttpSendRequest")?;
    unsafe { WinHttpReceiveResponse(request.0, std::ptr::null_mut()) }
        .context("WinHttpReceiveResponse")?;

    let status = status_of(&request)?;
    if !(200..300).contains(&status) {
        // Deliberately without the body. An error answer from a documentation
        // host is an HTML page, and pouring the first 300 characters of
        // "<!DOCTYPE html><head>…" into the window says nothing and pushes
        // everything else off screen — which is exactly what it did.
        bail!("{status}");
    }

    body_of(&request)
}

/// Fetches a result the service pointed at.
pub fn download(url: &str) -> Result<Vec<u8>> {
    let target = Url::parse(url)?;

    let session = Handle::session()?;
    unsafe { WinHttpSetTimeouts(session.0, 10_000, 15_000, 60_000, 120_000) }
        .context("WinHttpSetTimeouts")?;
    let connect = Handle::connect(&session, &target)?;
    let request = Handle::request(&connect, &target, "GET")?;

    unsafe { WinHttpSendRequest(request.0, None, None, 0, 0, 0) }.context("WinHttpSendRequest")?;
    unsafe { WinHttpReceiveResponse(request.0, std::ptr::null_mut()) }
        .context("WinHttpReceiveResponse")?;

    let status = status_of(&request)?;
    if !(200..300).contains(&status) {
        bail!("\x1eAbruf antwortete mit {status}\x1fthe download answered {status}\x1d");
    }

    body_of(&request)
}

fn status_of(request: &Handle) -> Result<u32> {
    let mut code = 0u32;
    let mut length = size_of::<u32>() as u32;

    unsafe {
        WinHttpQueryHeaders(
            request.0,
            WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
            PCWSTR::null(),
            Some(&mut code as *mut u32 as *mut c_void),
            &mut length,
            std::ptr::null_mut(),
        )
    }
    .context("WinHttpQueryHeaders (Status)")?;

    Ok(code)
}

/// Every response header, taken as one block and split here.
///
/// One query instead of one per header: the block is what WinHTTP has anyway,
/// and asking by name would mean knowing the names in advance — which for a
/// `Location` that only some services send is exactly what we do not.
fn headers_of(request: &Handle) -> Result<Vec<(String, String)>> {
    let mut length = 0u32;

    // The first call is expected to fail with "buffer too small"; that is how
    // the size is asked for.
    let probe = unsafe {
        WinHttpQueryHeaders(
            request.0,
            WINHTTP_QUERY_RAW_HEADERS_CRLF,
            PCWSTR::null(),
            None,
            &mut length,
            std::ptr::null_mut(),
        )
    };
    if probe.is_ok() || length == 0 {
        return Ok(Vec::new());
    }

    let mut buffer = vec![0u16; length as usize / 2 + 2];
    unsafe {
        WinHttpQueryHeaders(
            request.0,
            WINHTTP_QUERY_RAW_HEADERS_CRLF,
            PCWSTR::null(),
            Some(buffer.as_mut_ptr() as *mut c_void),
            &mut length,
            std::ptr::null_mut(),
        )
    }
    .context("WinHttpQueryHeaders (Kopfzeilen)")?;

    let text = String::from_utf16_lossy(&buffer[..length as usize / 2]);
    Ok(text
        .split("\r\n")
        .filter_map(|line| line.split_once(':'))
        .map(|(name, value)| (name.trim().to_string(), value.trim().to_string()))
        .collect())
}

fn body_of(request: &Handle) -> Result<Vec<u8>> {
    let mut body = Vec::new();
    let mut chunk = vec![0u8; 64 * 1024];

    loop {
        let mut read = 0u32;
        unsafe {
            WinHttpReadData(
                request.0,
                chunk.as_mut_ptr() as *mut c_void,
                chunk.len() as u32,
                &mut read,
            )
        }
        .context("WinHttpReadData")?;

        if read == 0 {
            break;
        }
        body.extend_from_slice(&chunk[..read as usize]);

        // A service that answers with a gigabyte is not one we are talking to
        // on purpose.
        if body.len() > 256 * 1024 * 1024 {
            bail!("\x1eAntwort zu groß\x1fanswer too large\x1d");
        }
    }

    Ok(body)
}

/// The pieces WinHTTP wants an address broken into.
struct Url {
    host: String,
    /// Path plus query, which WinHTTP hands back separately and the request
    /// needs as one.
    object: String,
    port: u16,
    secure: bool,
}

impl Url {
    fn parse(url: &str) -> Result<Self> {
        // No trailing NUL: this binding passes the slice length to WinHTTP as
        // the string length, so a NUL would be data and the scheme would come
        // out unrecognised.
        let wide: Vec<u16> = url.encode_utf16().collect();

        let mut parts = URL_COMPONENTS {
            dwStructSize: size_of::<URL_COMPONENTS>() as u32,
            // Lengths of u32::MAX with null pointers mean: point into my
            // buffer rather than copying into one of yours.
            dwHostNameLength: u32::MAX,
            dwUrlPathLength: u32::MAX,
            dwExtraInfoLength: u32::MAX,
            dwSchemeLength: u32::MAX,
            ..Default::default()
        };

        unsafe { WinHttpCrackUrl(&wide, 0, &mut parts) }.with_context(|| {
            format!("\x1eAdresse nicht lesbar\x1fcannot read address\x1d: {url}")
        })?;

        // Safe because `wide` outlives every read here: WinHTTP filled in
        // pointers into that very buffer.
        let piece = |pointer: windows::core::PWSTR, length: u32| -> String {
            if pointer.is_null() || length == 0 {
                String::new()
            } else {
                unsafe {
                    String::from_utf16_lossy(std::slice::from_raw_parts(pointer.0, length as usize))
                }
            }
        };

        let host = piece(parts.lpszHostName, parts.dwHostNameLength);
        if host.is_empty() {
            bail!("\x1eAdresse ohne Rechnernamen\x1faddress without a host\x1d: {url}");
        }

        let path = piece(parts.lpszUrlPath, parts.dwUrlPathLength);
        let extra = piece(parts.lpszExtraInfo, parts.dwExtraInfoLength);
        let object = match (path.is_empty(), extra.is_empty()) {
            (true, true) => "/".to_string(),
            _ => format!("{path}{extra}"),
        };

        let secure = url.to_ascii_lowercase().starts_with("https://");
        let port = if parts.nPort != 0 {
            parts.nPort
        } else if secure {
            INTERNET_DEFAULT_HTTPS_PORT
        } else {
            80
        };

        Ok(Url {
            host,
            object,
            port,
            secure,
        })
    }
}

/// A WinHTTP handle that closes itself.
struct Handle(*mut c_void);

impl Handle {
    fn session() -> Result<Self> {
        let agent: Vec<u16> = "ctxmenu".encode_utf16().chain(std::iter::once(0)).collect();

        // AUTOMATIC_PROXY takes the settings the user already has; a program
        // that ignores the company proxy simply does not reach anything.
        let handle = unsafe {
            WinHttpOpen(
                PCWSTR(agent.as_ptr()),
                WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
                PCWSTR::null(),
                PCWSTR::null(),
                0,
            )
        };
        Self::check(handle, "WinHttpOpen")
    }

    fn connect(session: &Handle, url: &Url) -> Result<Self> {
        let host: Vec<u16> = url.host.encode_utf16().chain(std::iter::once(0)).collect();

        let handle = unsafe { WinHttpConnect(session.0, PCWSTR(host.as_ptr()), url.port, 0) };
        Self::check(handle, "WinHttpConnect")
    }

    fn request(connect: &Handle, url: &Url, method: &str) -> Result<Self> {
        let verb: Vec<u16> = method
            .to_ascii_uppercase()
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        let object: Vec<u16> = url
            .object
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();

        let flags = if url.secure {
            WINHTTP_FLAG_SECURE
        } else {
            WINHTTP_OPEN_REQUEST_FLAGS(0)
        };

        let handle = unsafe {
            WinHttpOpenRequest(
                connect.0,
                PCWSTR(verb.as_ptr()),
                PCWSTR(object.as_ptr()),
                PCWSTR::null(),
                PCWSTR::null(),
                std::ptr::null(),
                flags,
            )
        };
        Self::check(handle, "WinHttpOpenRequest")
    }

    /// The three handle-making calls answer with a raw pointer and no result;
    /// NULL is the failure, and the reason has to be fetched separately.
    fn check(handle: *mut c_void, what: &str) -> Result<Self> {
        if handle.is_null() {
            Err(anyhow::Error::from(windows::core::Error::from_thread()).context(what.to_string()))
        } else {
            Ok(Handle(handle))
        }
    }
}

impl Drop for Handle {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                let _ = WinHttpCloseHandle(self.0);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_address_is_taken_apart_the_way_winhttp_needs_it() {
        let url = Url::parse("https://api.tinify.com/shrink").expect("parses");
        assert_eq!(url.host, "api.tinify.com");
        assert_eq!(url.object, "/shrink");
        assert_eq!(url.port, 443);
        assert!(url.secure);

        // Path and query arrive separately and must be put back together, or
        // the query is silently dropped from the request.
        let url = Url::parse("https://tool.example/api/v2/upload?format=png&q=80").expect("parses");
        assert_eq!(url.object, "/api/v2/upload?format=png&q=80");

        let url = Url::parse("http://192.168.1.10:8080/").expect("parses");
        assert_eq!(url.host, "192.168.1.10");
        assert_eq!(url.port, 8080);
        assert!(!url.secure);

        // No path at all still needs one.
        let url = Url::parse("https://example.org").expect("parses");
        assert_eq!(url.object, "/");
    }

    #[test]
    fn nonsense_addresses_are_refused_rather_than_dialled() {
        assert!(Url::parse("nicht mal eine adresse").is_err());
        assert!(Url::parse("https://").is_err());
    }

    #[test]
    fn a_multipart_body_carries_the_file_and_closes_its_boundary() {
        let extra = vec![Header {
            name: "quality".into(),
            value: "80".into(),
        }];
        let request = Request::multipart(
            "file",
            "Bild \"1\".png",
            b"ABCDEF".to_vec(),
            "image/png",
            &extra,
        );

        let text = String::from_utf8_lossy(&request.body);
        let boundary = request
            .content_type
            .split("boundary=")
            .nth(1)
            .expect("boundary is announced")
            .to_string();

        assert!(request.content_type.starts_with("multipart/form-data;"));
        assert!(text.contains("name=\"quality\"\r\n\r\n80\r\n"), "{text}");
        assert!(
            text.contains("filename=\"Bild \\\"1\\\".png\""),
            "the quote must be escaped: {text}"
        );
        assert!(text.contains("Content-Type: image/png"));
        assert!(
            request.body.windows(6).any(|w| w == b"ABCDEF"),
            "the file itself must be in there"
        );
        assert!(
            text.ends_with(&format!("--{boundary}--\r\n")),
            "the closing boundary is missing"
        );

        // The separator must not occur inside the payload, or the parts split
        // in the wrong place.
        assert_eq!(
            text.matches(&boundary).count(),
            3,
            "opening, file part, closing"
        );
    }

    #[test]
    fn a_raw_body_is_exactly_the_file() {
        let request = Request::raw(b"\x89PNG\r\n".to_vec(), "image/png");
        assert_eq!(request.body, b"\x89PNG\r\n");
        assert_eq!(request.content_type, "image/png");
    }

    #[test]
    fn boundaries_differ_between_calls() {
        let a = boundary();
        let b = boundary();
        assert_ne!(a, b, "two uploads must not share a separator");
        assert!(a.starts_with("----ctxmenu"));
    }

    #[test]
    fn a_session_can_be_opened_and_closes_itself() {
        // Proves the binding and the feature flag, without touching a network.
        let session = Handle::session().expect("WinHTTP is present on every Windows");
        assert!(!session.0.is_null());
    }
}
