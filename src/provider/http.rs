//! Shared HTTP helpers for provider adapters.
use crate::{
    ai::ServiceError,
    credentials::redact,
    json::{self, JsonLimits, JsonValue},
    transport::{Response, TransportError, TransportErrorKind},
};

const MAX_ERROR_EXCERPT: usize = 512;

/// Convert a transport failure into a redacted provider error.
pub(super) fn transport_error(error: TransportError, secrets: &[&str]) -> ServiceError {
    let message = match error.kind() {
        TransportErrorKind::Timeout => "provider request timed out".to_owned(),
        TransportErrorKind::Connect => "cannot reach the provider".to_owned(),
        TransportErrorKind::Tls => "provider TLS negotiation failed".to_owned(),
        TransportErrorKind::Redirect => "the provider redirected the request".to_owned(),
        TransportErrorKind::Body => "the provider response exceeded its bound".to_owned(),
        TransportErrorKind::InvalidRequest => "the provider request was invalid".to_owned(),
        TransportErrorKind::Protocol => {
            format!("provider transport failed: {}", error.message())
        }
    };
    ServiceError::provider(redact(secrets.iter().copied(), &message))
}

/// Require a 2xx status, redacting a bounded body excerpt on failure.
pub(super) fn require_success(
    response: Response,
    secrets: &[&str],
) -> std::result::Result<Response, ServiceError> {
    if response.is_success() {
        return Ok(response);
    }
    let body = String::from_utf8_lossy(&response.body);
    let excerpt = excerpt(&redact(secrets.iter().copied(), &body), MAX_ERROR_EXCERPT);
    Err(ServiceError::provider(format!(
        "provider returned HTTP {}: {excerpt}",
        response.status
    )))
}

/// Parse a bounded JSON body, mapping failures to a provider error.
pub(super) fn json_body(response: &Response) -> std::result::Result<JsonValue, ServiceError> {
    json::parse(&response.body, &JsonLimits::default()).map_err(|error| {
        ServiceError::provider(format!(
            "provider body is not valid JSON: {}",
            error.message()
        ))
    })
}

fn excerpt(text: &str, max: usize) -> String {
    let mut out = String::new();
    for ch in text.chars() {
        if out.len() + ch.len_utf8() > max {
            out.push('…');
            break;
        }
        if ch.is_control() {
            out.push(' ');
        } else {
            out.push(ch);
        }
    }
    out
}
