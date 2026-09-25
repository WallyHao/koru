//! Shared Chat Completions agent loop used by the provider adapters.
use super::{http, protocol};
use crate::{
    ai::{AiRequest, AiResult, FinishReason, ServiceError, ServiceEvents, Usage},
    json,
    transport::{Request, Transport},
};

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
        let response = transport
            .send(&endpoint.request(&body))
            .map_err(|error| http::transport_error(error, &endpoint.secrets()))?;
        let response = http::require_success(response, &endpoint.secrets())?;
        let document = http::json_body(&response)?;
        let parsed = protocol::parse_response(&document)
            .map_err(|error| ServiceError::provider(error.message().to_owned()))?;
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
