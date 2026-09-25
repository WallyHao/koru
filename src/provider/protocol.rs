//! Chat Completions payload mapping over the Koru JSON model.
//!
//! Request and response shapes are pure JSON transformations so adapters can be
//! tested without a network. Tool arguments are exchanged as emitted JSON
//! strings, exactly as the protocol expects, and are parsed back under the
//! bounded JSON limits.
use crate::{
    ai::{FinishReason, ToolCall, ToolResult, ToolSpec, Usage},
    error::{ErrorCode, KoruError, Result},
    json::{self, JsonLimits, JsonValue},
};
use std::collections::{BTreeMap, BTreeSet};

/// Maximum bytes in a provider-assigned tool call identity.
pub const MAX_CALL_ID_BYTES: usize = 256;
/// Field used for the requested effort variant, when the adapter maps one.
pub const EFFORT_FIELD: &str = "reasoning_effort";

/// The ordered messages of one conversation.
#[derive(Debug, Clone, Default)]
pub struct Conversation {
    messages: Vec<JsonValue>,
}
impl Conversation {
    /// Start a conversation with the workflow prompt as the user message.
    pub fn new(prompt: &str) -> Self {
        Self {
            messages: vec![message("user", Some(prompt))],
        }
    }
    /// Append the assistant turn that requested the given tool calls.
    pub fn push_assistant(&mut self, text: Option<&str>, calls: &[ToolCall]) {
        let mut entry = object(vec![
            ("role", JsonValue::String("assistant".to_owned())),
            (
                "content",
                match text {
                    Some(text) => JsonValue::String(text.to_owned()),
                    None => JsonValue::Null,
                },
            ),
        ]);
        if !calls.is_empty() {
            let payload = calls
                .iter()
                .map(|call| {
                    object(vec![
                        ("id", JsonValue::String(call.id.clone())),
                        ("type", JsonValue::String("function".to_owned())),
                        (
                            "function",
                            object(vec![
                                ("name", JsonValue::String(call.name.clone())),
                                ("arguments", JsonValue::String(json::emit(&call.arguments))),
                            ]),
                        ),
                    ])
                })
                .collect();
            set(&mut entry, "tool_calls", JsonValue::Array(payload));
        }
        self.messages.push(entry);
    }
    /// Append one tool result for the given call identity.
    pub fn push_tool_result(&mut self, call_id: &str, result: &ToolResult) {
        self.messages.push(object(vec![
            ("role", JsonValue::String("tool".to_owned())),
            ("tool_call_id", JsonValue::String(call_id.to_owned())),
            ("content", JsonValue::String(json::emit(&result.content))),
        ]));
    }
    /// The messages accumulated so far.
    pub fn messages(&self) -> &[JsonValue] {
        &self.messages
    }
}

/// Build a Chat Completions request body.
pub fn build_request(
    model: &str,
    conversation: &Conversation,
    tools: &[ToolSpec],
    effort: Option<&str>,
) -> JsonValue {
    let mut request = object(vec![
        ("model", JsonValue::String(model.to_owned())),
        (
            "messages",
            JsonValue::Array(conversation.messages().to_vec()),
        ),
        ("stream", JsonValue::Bool(false)),
    ]);
    if !tools.is_empty() {
        set(
            &mut request,
            "tools",
            JsonValue::Array(tools.iter().map(tool_payload).collect()),
        );
    }
    if let Some(effort) = effort {
        set(
            &mut request,
            EFFORT_FIELD,
            JsonValue::String(effort.to_owned()),
        );
    }
    request
}

fn tool_payload(spec: &ToolSpec) -> JsonValue {
    object(vec![
        ("type", JsonValue::String("function".to_owned())),
        (
            "function",
            object(vec![
                ("name", JsonValue::String(spec.name.clone())),
                ("description", JsonValue::String(spec.description.clone())),
                ("parameters", spec.parameters.document().clone()),
            ]),
        ),
    ])
}

/// One parsed model response.
#[derive(Debug, Clone)]
pub struct ParsedResponse {
    /// Assistant text, empty when the model only requested tools.
    pub text: String,
    /// Tool calls in response order.
    pub calls: Vec<ToolCall>,
    /// Normalized finish reason.
    pub finish: FinishReason,
    /// Reported token usage, when present.
    pub usage: Usage,
    /// Provider request identity, when present.
    pub request_id: Option<String>,
}

/// Parse a Chat Completions response document.
pub fn parse_response(document: &JsonValue) -> Result<ParsedResponse> {
    let root = as_object(document, "response")?;
    let request_id = match root.get("id") {
        Some(JsonValue::String(id)) => Some(id.clone()),
        Some(_) => return Err(provider("response `id` must be a string")),
        None => None,
    };
    let choices = match root.get("choices") {
        Some(JsonValue::Array(choices)) if !choices.is_empty() => choices,
        _ => return Err(provider("response has no `choices`")),
    };
    let choice = as_object(&choices[0], "choice")?;
    let message = choice
        .get("message")
        .ok_or_else(|| provider("response choice has no `message`"))?;
    let message = as_object(message, "message")?;
    let text = match message.get("content") {
        Some(JsonValue::String(text)) => text.clone(),
        Some(JsonValue::Null) | None => String::new(),
        Some(_) => return Err(provider("message `content` must be a string or null")),
    };
    let mut calls = Vec::new();
    let mut seen = BTreeSet::new();
    if let Some(tool_calls) = message.get("tool_calls") {
        let JsonValue::Array(tool_calls) = tool_calls else {
            return Err(provider("message `tool_calls` must be an array"));
        };
        for entry in tool_calls {
            let call = parse_call(entry)?;
            if !seen.insert(call.id.clone()) {
                return Err(provider("duplicate tool call id in response"));
            }
            calls.push(call);
        }
    }
    let finish = match choice.get("finish_reason") {
        Some(JsonValue::String(reason)) => finish_reason(reason),
        Some(_) => return Err(provider("`finish_reason` must be a string")),
        None => FinishReason::Other("unknown".to_owned()),
    };
    if text.is_empty() && calls.is_empty() {
        return Err(provider("response has neither content nor tool calls"));
    }
    let usage = parse_usage(root.get("usage"))?;
    Ok(ParsedResponse {
        text,
        calls,
        finish,
        usage,
        request_id,
    })
}

fn parse_call(value: &JsonValue) -> Result<ToolCall> {
    let call = as_object(value, "tool call")?;
    let id = match call.get("id") {
        Some(JsonValue::String(id)) if !id.is_empty() && id.len() <= MAX_CALL_ID_BYTES => {
            id.clone()
        }
        Some(JsonValue::String(_)) => {
            return Err(provider("tool call `id` has an invalid length"));
        }
        _ => return Err(provider("tool call has no string `id`")),
    };
    let function = call
        .get("function")
        .ok_or_else(|| provider("tool call has no `function`"))?;
    let function = as_object(function, "tool function")?;
    let name = match function.get("name") {
        Some(JsonValue::String(name)) if !name.is_empty() => name.clone(),
        _ => return Err(provider("tool call function has no `name`")),
    };
    let arguments = match function.get("arguments") {
        Some(JsonValue::String(text)) => text,
        Some(JsonValue::Object(_)) => {
            return Err(provider(
                "tool call `arguments` must be a JSON string, not an object",
            ));
        }
        _ => return Err(provider("tool call function has no `arguments` string")),
    };
    let parsed = json::parse(arguments.as_bytes(), &JsonLimits::default()).map_err(|error| {
        provider(format!(
            "tool call arguments are not valid JSON: {}",
            error.message()
        ))
    })?;
    if !matches!(parsed, JsonValue::Object(_)) {
        return Err(provider("tool call arguments must be a JSON object"));
    }
    Ok(ToolCall {
        id,
        name,
        arguments: parsed,
    })
}

fn parse_usage(value: Option<&JsonValue>) -> Result<Usage> {
    let Some(value) = value else {
        return Ok(Usage::default());
    };
    let usage = as_object(value, "usage")?;
    Ok(Usage {
        input_tokens: token_count(usage.get("prompt_tokens"), "prompt_tokens")?,
        output_tokens: token_count(usage.get("completion_tokens"), "completion_tokens")?,
    })
}

fn token_count(value: Option<&JsonValue>, field: &str) -> Result<Option<u64>> {
    match value {
        Some(JsonValue::Integer(count)) if *count >= 0 => Ok(Some(*count as u64)),
        Some(_) => Err(provider(format!(
            "usage `{field}` must be a nonnegative integer"
        ))),
        None => Ok(None),
    }
}

fn finish_reason(reason: &str) -> FinishReason {
    match reason {
        "stop" => FinishReason::Stop,
        "length" => FinishReason::Length,
        "tool_calls" | "function_call" => FinishReason::ToolCalls,
        "content_filter" => FinishReason::ContentFilter,
        other => FinishReason::Other(other.to_owned()),
    }
}

fn as_object<'a>(value: &'a JsonValue, what: &str) -> Result<&'a BTreeMap<String, JsonValue>> {
    match value {
        JsonValue::Object(entries) => Ok(entries),
        _ => Err(provider(format!("response `{what}` must be an object"))),
    }
}

fn object(pairs: Vec<(&str, JsonValue)>) -> JsonValue {
    JsonValue::Object(
        pairs
            .into_iter()
            .map(|(key, value)| (key.to_owned(), value))
            .collect(),
    )
}

fn set(target: &mut JsonValue, key: &str, value: JsonValue) {
    if let JsonValue::Object(entries) = target {
        entries.insert(key.to_owned(), value);
    }
}

fn message(role: &str, content: Option<&str>) -> JsonValue {
    object(vec![
        ("role", JsonValue::String(role.to_owned())),
        (
            "content",
            match content {
                Some(content) => JsonValue::String(content.to_owned()),
                None => JsonValue::Null,
            },
        ),
    ])
}

fn provider(detail: impl Into<String>) -> KoruError {
    KoruError::new(
        ErrorCode::ProviderFailure,
        format!("invalid provider response: {}", detail.into()),
    )
}
