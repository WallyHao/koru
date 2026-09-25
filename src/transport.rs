//! Blocking HTTP transport boundary; adapters never open sockets directly.
//!
//! The production transport is `ureq` over rustls with pinned webpki roots. Any
//! HTTP status is returned as a [`Response`]; only transport-level failures (DNS,
//! connect, TLS, protocol, size, redirect) become a classified
//! [`TransportError`]. Redirects are disabled so credentials can never be
//! forwarded to another host. Bodies are bounded on both directions.
use crate::error::{ErrorCode, KoruError};
use std::{collections::VecDeque, sync::Mutex, time::Duration};

/// HTTP method understood by the transport.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    /// A bodyless GET.
    Get,
    /// A POST with a bounded body.
    Post,
}

/// A bounded outbound request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    /// Request method.
    pub method: Method,
    /// Absolute URL; credentials never appear in it.
    pub url: String,
    /// Request headers in order.
    pub headers: Vec<(String, String)>,
    /// Request body; empty for `GET`.
    pub body: Vec<u8>,
}
impl Request {
    /// A GET request with no body.
    pub fn get(url: impl Into<String>) -> Self {
        Self {
            method: Method::Get,
            url: url.into(),
            headers: Vec::new(),
            body: Vec::new(),
        }
    }
    /// A POST request with the given body.
    pub fn post(url: impl Into<String>, body: Vec<u8>) -> Self {
        Self {
            method: Method::Post,
            url: url.into(),
            headers: Vec::new(),
            body,
        }
    }
    /// Add one header.
    pub fn with_header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.push((name.into(), value.into()));
        self
    }
    /// Return the first header value matching `name`, case-insensitively.
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }
}

/// A bounded response; every HTTP status is represented, including errors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Response {
    /// HTTP status code.
    pub status: u16,
    /// Response headers.
    pub headers: Vec<(String, String)>,
    /// Response body.
    pub body: Vec<u8>,
}
impl Response {
    /// A response with a text body.
    pub fn text(status: u16, body: impl Into<String>) -> Self {
        Self {
            status,
            headers: Vec::new(),
            body: body.into().into_bytes(),
        }
    }
    /// Whether the status is in the 2xx range.
    pub fn is_success(&self) -> bool {
        (200..300).contains(&self.status)
    }
    /// Return the first header value matching `name`, case-insensitively.
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }
    /// Parse the body as UTF-8, failing as a transport body error.
    pub fn text_body(&self) -> std::result::Result<&str, TransportError> {
        std::str::from_utf8(&self.body)
            .map_err(|_| TransportError::new(TransportErrorKind::Body, "response is not UTF-8"))
    }
}

/// Transport-level failure classes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportErrorKind {
    /// A configured timeout elapsed.
    Timeout,
    /// The connection could not be established.
    Connect,
    /// TLS negotiation or certificate validation failed.
    Tls,
    /// The HTTP exchange was malformed.
    Protocol,
    /// A body exceeded its bound or was not valid for its content.
    Body,
    /// A redirect was returned or attempted.
    Redirect,
    /// The request itself was invalid.
    InvalidRequest,
}

/// A classified transport failure.
#[derive(Debug, Clone)]
pub struct TransportError {
    kind: TransportErrorKind,
    message: String,
}
impl TransportError {
    /// Build a transport failure.
    pub fn new(kind: TransportErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }
    /// The failure class.
    pub fn kind(&self) -> TransportErrorKind {
        self.kind
    }
    /// A bounded, credential-free message.
    pub fn message(&self) -> &str {
        &self.message
    }
    /// Convert to a stable Koru error.
    pub fn into_koru(self) -> KoruError {
        let code = match self.kind {
            TransportErrorKind::Timeout => ErrorCode::Timeout,
            _ => ErrorCode::ProviderFailure,
        };
        KoruError::new(code, self.message)
    }
}
impl std::fmt::Display for TransportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

/// Bounds applied to every request.
#[derive(Debug, Clone, Copy)]
pub struct TransportLimits {
    /// Maximum time to establish a connection.
    pub connect_timeout: Duration,
    /// Maximum time to receive a response.
    pub read_timeout: Duration,
    /// Maximum accepted response body size.
    pub max_response_bytes: u64,
    /// Maximum accepted request body size.
    pub max_request_bytes: usize,
    /// Maximum redirects; Koru keeps this at zero.
    pub max_redirects: u32,
}
impl Default for TransportLimits {
    fn default() -> Self {
        Self {
            connect_timeout: Duration::from_secs(10),
            read_timeout: Duration::from_secs(180),
            max_response_bytes: 16 * 1024 * 1024,
            max_request_bytes: 4 * 1024 * 1024,
            max_redirects: 0,
        }
    }
}

/// The blocking transport boundary.
pub trait Transport: Send + Sync {
    /// Send one request and return its bounded response.
    fn send(&self, request: &Request) -> std::result::Result<Response, TransportError>;
}

impl<T: Transport + ?Sized> Transport for std::sync::Arc<T> {
    fn send(&self, request: &Request) -> std::result::Result<Response, TransportError> {
        (**self).send(request)
    }
}

/// The production transport backed by `ureq` and rustls.
pub struct UreqTransport {
    agent: ureq::Agent,
    limits: TransportLimits,
}
impl UreqTransport {
    /// Build the transport with the given bounds.
    pub fn new(limits: TransportLimits) -> Self {
        let config = ureq::Agent::config_builder()
            .timeout_connect(Some(limits.connect_timeout))
            .timeout_recv_response(Some(limits.read_timeout))
            .timeout_global(Some(limits.read_timeout))
            .max_redirects(limits.max_redirects)
            .http_status_as_error(false)
            .user_agent(concat!("koru/", env!("CARGO_PKG_VERSION")))
            .build();
        Self {
            agent: ureq::Agent::new_with_config(config),
            limits,
        }
    }
}
impl Transport for UreqTransport {
    fn send(&self, request: &Request) -> std::result::Result<Response, TransportError> {
        if request.body.len() > self.limits.max_request_bytes {
            return Err(TransportError::new(
                TransportErrorKind::InvalidRequest,
                "request body exceeds the transport limit",
            ));
        }
        let result = match request.method {
            Method::Get => {
                let mut builder = self.agent.get(request.url.as_str());
                for (name, value) in &request.headers {
                    builder = builder.header(name.as_str(), value.as_str());
                }
                builder.call()
            }
            Method::Post => {
                let mut builder = self.agent.post(request.url.as_str());
                for (name, value) in &request.headers {
                    builder = builder.header(name.as_str(), value.as_str());
                }
                builder.send(request.body.as_slice())
            }
        };
        let mut response = result.map_err(classify)?;
        let status = response.status().as_u16();
        let headers = response
            .headers()
            .iter()
            .filter_map(|(name, value)| {
                value
                    .to_str()
                    .ok()
                    .map(|value| (name.as_str().to_owned(), value.to_owned()))
            })
            .collect();
        let body = response
            .body_mut()
            .with_config()
            .limit(self.limits.max_response_bytes + 1)
            .read_to_vec()
            .map_err(classify)?;
        if body.len() as u64 > self.limits.max_response_bytes {
            return Err(TransportError::new(
                TransportErrorKind::Body,
                "response body exceeds the transport limit",
            ));
        }
        Ok(Response {
            status,
            headers,
            body,
        })
    }
}

fn classify(error: ureq::Error) -> TransportError {
    use ureq::Error;
    let (kind, message) = match error {
        Error::Timeout(_) => (TransportErrorKind::Timeout, "request timed out".to_owned()),
        Error::Io(ref io) if io.kind() == std::io::ErrorKind::TimedOut => {
            (TransportErrorKind::Timeout, "request timed out".to_owned())
        }
        Error::HostNotFound => (
            TransportErrorKind::Connect,
            "host could not be resolved".to_owned(),
        ),
        Error::ConnectionFailed | Error::Io(_) => {
            (TransportErrorKind::Connect, "connection failed".to_owned())
        }
        Error::Tls(_) | Error::Rustls(_) | Error::Pem(_) => {
            (TransportErrorKind::Tls, "TLS negotiation failed".to_owned())
        }
        Error::RedirectFailed | Error::TooManyRedirects => (
            TransportErrorKind::Redirect,
            "the server redirected the request".to_owned(),
        ),
        Error::BodyExceedsLimit(_) => (
            TransportErrorKind::Body,
            "response body exceeds the transport limit".to_owned(),
        ),
        Error::BadUri(_) | Error::InvalidProxyUrl => (
            TransportErrorKind::InvalidRequest,
            "invalid request URL".to_owned(),
        ),
        Error::StatusCode(status) => (
            TransportErrorKind::Protocol,
            format!("unexpected HTTP status {status}"),
        ),
        Error::Http(_) | Error::Protocol(_) => (
            TransportErrorKind::Protocol,
            "HTTP exchange failed".to_owned(),
        ),
        _ => (TransportErrorKind::Protocol, "transport failure".to_owned()),
    };
    TransportError::new(kind, message)
}

/// A deterministic, replayable transport used by tests.
pub struct FixtureTransport {
    limits: TransportLimits,
    scripted: Mutex<VecDeque<std::result::Result<Response, TransportError>>>,
    requests: Mutex<Vec<Request>>,
}
impl FixtureTransport {
    /// A transport with no scripted responses.
    pub fn new(limits: TransportLimits) -> Self {
        Self {
            limits,
            scripted: Mutex::new(VecDeque::new()),
            requests: Mutex::new(Vec::new()),
        }
    }
    /// Queue a successful response.
    pub fn push(&self, response: Response) {
        self.scripted
            .lock()
            .expect("fixture transport poisoned")
            .push_back(Ok(response));
    }
    /// Queue a failure.
    pub fn push_error(&self, kind: TransportErrorKind, message: impl Into<String>) {
        self.scripted
            .lock()
            .expect("fixture transport poisoned")
            .push_back(Err(TransportError::new(kind, message)));
    }
    /// Snapshot the requests received so far.
    pub fn requests(&self) -> Vec<Request> {
        self.requests
            .lock()
            .expect("fixture transport poisoned")
            .clone()
    }
}
impl Transport for FixtureTransport {
    fn send(&self, request: &Request) -> std::result::Result<Response, TransportError> {
        if request.body.len() > self.limits.max_request_bytes {
            return Err(TransportError::new(
                TransportErrorKind::InvalidRequest,
                "request body exceeds the transport limit",
            ));
        }
        self.requests
            .lock()
            .expect("fixture transport poisoned")
            .push(request.clone());
        self.scripted
            .lock()
            .expect("fixture transport poisoned")
            .pop_front()
            .unwrap_or_else(|| {
                Err(TransportError::new(
                    TransportErrorKind::Protocol,
                    "no scripted response",
                ))
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_ureq_failures() {
        assert_eq!(
            classify(ureq::Error::HostNotFound).kind(),
            TransportErrorKind::Connect
        );
        assert_eq!(
            classify(ureq::Error::TooManyRedirects).kind(),
            TransportErrorKind::Redirect
        );
        assert_eq!(
            classify(ureq::Error::BadUri("x".to_owned())).kind(),
            TransportErrorKind::InvalidRequest
        );
        assert_eq!(
            classify(ureq::Error::Tls("bad cert")).kind(),
            TransportErrorKind::Tls
        );
    }

    #[test]
    fn maps_transport_errors_to_stable_codes() {
        let timeout = TransportError::new(TransportErrorKind::Timeout, "late");
        assert_eq!(timeout.into_koru().code(), ErrorCode::Timeout);
        let connect = TransportError::new(TransportErrorKind::Connect, "down");
        assert_eq!(connect.into_koru().code(), ErrorCode::ProviderFailure);
    }

    #[test]
    fn default_limits_never_redirect() {
        assert_eq!(TransportLimits::default().max_redirects, 0);
    }
}
