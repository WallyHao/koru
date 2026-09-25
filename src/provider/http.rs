//! Shared HTTP helpers for provider adapters.
use crate::{
    ai::ServiceError,
    credentials::redact,
    json::{self, JsonLimits, JsonValue},
    transport::{Response, TransportError, TransportErrorKind},
};
use std::time::Duration;

const MAX_ERROR_EXCERPT: usize = 512;
/// Longest honored `Retry-After` delay.
pub(super) const MAX_RETRY_AFTER_SECS: u64 = 5;

/// A classified non-2xx provider response.
pub(super) struct HttpFailure {
    /// The normalized provider error.
    pub error: ServiceError,
    /// Whether a retry within the same budget could reasonably succeed.
    pub retryable: bool,
    /// A bounded server-requested delay.
    pub retry_after: Option<Duration>,
}

/// Classify a non-2xx response for retry decisions.
pub(super) fn failure_from_response(response: &Response, secrets: &[&str]) -> HttpFailure {
    let retryable = matches!(response.status, 408 | 429 | 500 | 502 | 503 | 504);
    let retry_after = response
        .header("retry-after")
        .and_then(|value| value.trim().parse::<u64>().ok())
        .map(|seconds| Duration::from_secs(seconds.min(MAX_RETRY_AFTER_SECS)));
    let body = String::from_utf8_lossy(&response.body);
    let excerpt = excerpt(&redact(secrets.iter().copied(), &body), MAX_ERROR_EXCERPT);
    HttpFailure {
        error: ServiceError::provider(format!(
            "provider returned HTTP {}: {excerpt}",
            response.status
        )),
        retryable,
        retry_after,
    }
}

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
