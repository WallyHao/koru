//! Shared Chat Completions agent loop used by the provider adapters.
use super::{http, protocol};
use crate::{
    ai::{AiRequest, AiResult, FinishReason, ServiceError, ServiceEvents, Usage},
    json,
    transport::{Request, Transport},
};
use std::time::Duration;

/// Bounded attempts for one model turn, independent of the shared budget.
const MAX_ATTEMPTS_PER_TURN: u32 = 3;

/// One fixed provider endpoint and selection.
pub(super) struct ChatEndpoint {
    /// Full chat completions URL.
    pub url: String,
    /// Selected model identity.
    pub model: String,
    /// Optional effort field value.
    pub effort: Option<String>,
    /// Bearer credential.
    pub credential: String,
    /// Optional session header as `(name, value)`.
    pub session: Option<(String, String)>,
}
impl ChatEndpoint {
    fn request(&self, body: &crate::json::JsonValue) -> Request {
        let mut request = Request::post(self.url.clone(), json::emit(body).into_bytes())
            .with_header("Authorization", format!("Bearer {}", self.credential))
            .with_header("Content-Type", "application/json")
            .with_header("Accept", "application/json");
        if let Some((name, value)) = &self.session {
            request = request.with_header(name.clone(), value.clone());
        }
        request
    }
    fn secrets(&self) -> [&str; 1] {
        [self.credential.as_str()]
    }
}

/// Run the bounded agent loop against one Chat Completions endpoint.
pub(super) fn run_chat(
    transport: &dyn Transport,
    endpoint: &ChatEndpoint,
    request: AiRequest,
    events: &mut dyn ServiceEvents,
) -> std::result::Result<AiResult, ServiceError> {
    let mut conversation = protocol::Conversation::new(&request.prompt);
    let mut text = String::new();
    let mut usage = Usage::default();
    let mut request_id = None;
    for turn in 0..request.max_turns {
        let body = protocol::build_request(
            &endpoint.model,
            &conversation,
            &request.tools,
            endpoint.effort.as_deref(),
        );
        let parsed = complete(transport, endpoint, &body, events)?;
        text = parsed.text.clone();
        usage = parsed.usage;
        request_id = parsed.request_id;
        if parsed.calls.is_empty() {
            return Ok(AiResult {
                text: parsed.text,
                finish_reason: parsed.finish,
                model: endpoint.model.clone(),
                usage,
                request_id,
            });
        }
        let assistant_text = (!parsed.text.is_empty()).then_some(parsed.text.as_str());
        conversation.push_assistant(assistant_text, &parsed.calls);
        for call in &parsed.calls {
            let result = events.call_tool(call.clone())?;
            conversation.push_tool_result(&call.id, &result);
        }
        if turn + 1 == request.max_turns {
            return Ok(AiResult {
                text,
                finish_reason: FinishReason::Length,
                model: endpoint.model.clone(),
                usage,
                request_id,
            });
        }
    }
    Ok(AiResult {
        text,
        finish_reason: FinishReason::Length,
        model: endpoint.model.clone(),
        usage,
        request_id,
    })
}

/// One failed model attempt with its retry classification.
struct AttemptFailure {
    error: ServiceError,
    retryable: bool,
    retry_after: Option<Duration>,
}

/// Complete one model turn, retrying transient failures within the shared budget.
fn complete(
    transport: &dyn Transport,
    endpoint: &ChatEndpoint,
    body: &crate::json::JsonValue,
    events: &mut dyn ServiceEvents,
) -> std::result::Result<protocol::ParsedResponse, ServiceError> {
    let mut attempt = 1;
    loop {
        match one_attempt(transport, endpoint, body) {
            Ok(parsed) => return Ok(parsed),
            Err(failure) if failure.retryable && attempt < MAX_ATTEMPTS_PER_TURN => {
                events.reserve_retry()?;
                if let Some(delay) = failure.retry_after {
                    std::thread::sleep(delay);
                }
                attempt += 1;
            }
            Err(failure) => return Err(failure.error),
        }
    }
}

fn one_attempt(
    transport: &dyn Transport,
    endpoint: &ChatEndpoint,
    body: &crate::json::JsonValue,
) -> std::result::Result<protocol::ParsedResponse, AttemptFailure> {
    let response = transport
        .send(&endpoint.request(body))
        .map_err(|error| AttemptFailure {
            retryable: error.retryable(),
            error: http::transport_error(error, &endpoint.secrets()),
            retry_after: None,
        })?;
    if !response.is_success() {
        let failure = http::failure_from_response(&response, &endpoint.secrets());
        return Err(AttemptFailure {
            error: failure.error,
            retryable: failure.retryable,
            retry_after: failure.retry_after,
        });
    }
    let document = http::json_body(&response).map_err(|error| AttemptFailure {
        error,
        retryable: false,
        retry_after: None,
    })?;
    protocol::parse_response(&document).map_err(|error| AttemptFailure {
        error: ServiceError::provider(error.message().to_owned()),
        retryable: false,
        retry_after: None,
    })
}

/// Build an authorized GET request for the given URL.
pub(super) fn authorized_get(
    url: String,
    credential: &str,
    session: Option<(&str, &str)>,
) -> Request {
    let mut request = Request::get(url)
        .with_header("Authorization", format!("Bearer {credential}"))
        .with_header("Accept", "application/json");
    if let Some((name, value)) = session {
        request = request.with_header(name, value);
    }
    request
}
