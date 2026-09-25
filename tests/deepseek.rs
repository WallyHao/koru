//! DeepSeek adapter: authorized requests, tool loop, errors, and catalog fetch.
use koru::{
    ai::{
        AiRequest, AiService, FinishReason, ServiceError, ServiceEvents, ToolCall, ToolResult,
        ToolSpec,
    },
    error::ErrorCode,
    json::{JsonLimits, JsonValue, parse},
    provider::{DeepSeekAdapter, ServiceId, catalog::CatalogSource},
    schema::JsonSchema,
    transport::{FixtureTransport, Response, TransportErrorKind, TransportLimits},
};
use std::sync::Arc;

fn spec() -> ToolSpec {
    let document = parse(
        br#"{"type":"object","properties":{"a":{"type":"integer"}}}"#,
        &JsonLimits::default(),
    )
    .unwrap();
    ToolSpec {
        name: "add".to_owned(),
        description: "adds".to_owned(),
        parameters: JsonSchema::compile(&document, &JsonLimits::default()).unwrap(),
    }
}

fn request(tools: Vec<ToolSpec>, max_turns: u32) -> AiRequest {
    AiRequest {
        prompt: "hello".to_owned(),
        tools,
        max_turns,
    }
}

struct Collector {
    calls: Vec<ToolCall>,
    content: JsonValue,
}
impl Default for Collector {
    fn default() -> Self {
        Self {
            calls: Vec::new(),
            content: JsonValue::Null,
        }
    }
}
impl ServiceEvents for Collector {
    fn call_tool(&mut self, call: ToolCall) -> Result<ToolResult, ServiceError> {
        self.calls.push(call);
        Ok(ToolResult {
            content: self.content.clone(),
        })
    }
}

fn adapter(transport: Arc<FixtureTransport>) -> DeepSeekAdapter {
    DeepSeekAdapter::new(transport, "fake-key", "deepseek-chat", None)
}

fn body_of(request: &koru::transport::Request) -> JsonValue {
    parse(&request.body, &JsonLimits::default()).unwrap()
}

#[test]
fn sends_an_authorized_chat_request() {
    let transport = Arc::new(FixtureTransport::new(TransportLimits::default()));
    transport.push(Response::text(
        200,
        r#"{"id":"req-1","choices":[{"message":{"content":"hi"},"finish_reason":"stop"}],"usage":{"prompt_tokens":3,"completion_tokens":2}}"#,
    ));
    let mut service = adapter(Arc::clone(&transport));
    let mut collector = Collector::default();
    let result = service
        .run(request(vec![spec()], 4), &mut collector)
        .unwrap();
    assert_eq!(result.text, "hi");
    assert_eq!(result.finish_reason, FinishReason::Stop);
    assert_eq!(result.model, "deepseek-chat");
    assert_eq!(result.usage.input_tokens, Some(3));
    assert_eq!(result.usage.output_tokens, Some(2));
    assert_eq!(result.request_id.as_deref(), Some("req-1"));

    let sent = transport.requests();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].url, "https://api.deepseek.com/chat/completions");
    assert_eq!(sent[0].header("authorization"), Some("Bearer fake-key"));
    assert_eq!(sent[0].header("content-type"), Some("application/json"));
    let body = body_of(&sent[0]);
    let JsonValue::Object(fields) = &body else {
        panic!("expected object");
    };
    assert_eq!(
        fields.get("model"),
        Some(&JsonValue::String("deepseek-chat".to_owned()))
    );
    assert!(matches!(fields.get("messages"), Some(JsonValue::Array(items)) if items.len() == 1));
    assert!(matches!(fields.get("tools"), Some(JsonValue::Array(items)) if items.len() == 1));
}

#[test]
fn runs_a_tool_loop() {
    let transport = Arc::new(FixtureTransport::new(TransportLimits::default()));
    transport.push(Response::text(
        200,
        r#"{"id":"req-1","choices":[{"message":{"content":null,"tool_calls":[{"id":"call-1","type":"function","function":{"name":"add","arguments":"{\"a\":1}"}}]},"finish_reason":"tool_calls"}]}"#,
    ));
    transport.push(Response::text(
        200,
        r#"{"id":"req-2","choices":[{"message":{"content":"done"},"finish_reason":"stop"}]}"#,
    ));
    let mut service = adapter(Arc::clone(&transport));
    let mut collector = Collector {
        calls: Vec::new(),
        content: parse(br#"{"sum":1}"#, &JsonLimits::default()).unwrap(),
    };
    let result = service
        .run(request(vec![spec()], 4), &mut collector)
        .unwrap();
    assert_eq!(result.text, "done");
    assert_eq!(collector.calls.len(), 1);
    assert_eq!(collector.calls[0].id, "call-1");
    assert_eq!(collector.calls[0].name, "add");
    assert_eq!(
        collector.calls[0].arguments,
        parse(br#"{"a":1}"#, &JsonLimits::default()).unwrap()
    );

    let sent = transport.requests();
    assert_eq!(sent.len(), 2);
    let body = body_of(&sent[1]);
    let JsonValue::Object(fields) = &body else {
        panic!("expected object");
    };
    let Some(JsonValue::Array(messages)) = fields.get("messages") else {
        panic!("expected messages");
    };
    assert_eq!(messages.len(), 3);
}

#[test]
fn stops_at_the_turn_limit_after_tool_calls() {
    let transport = Arc::new(FixtureTransport::new(TransportLimits::default()));
    transport.push(Response::text(
        200,
        r#"{"choices":[{"message":{"content":null,"tool_calls":[{"id":"call-1","type":"function","function":{"name":"add","arguments":"{}"}}]},"finish_reason":"tool_calls"}]}"#,
    ));
    let mut service = adapter(Arc::clone(&transport));
    let mut collector = Collector::default();
    let result = service
        .run(request(vec![spec()], 1), &mut collector)
        .unwrap();
    assert_eq!(result.finish_reason, FinishReason::Length);
    assert_eq!(collector.calls.len(), 1);
    assert_eq!(transport.requests().len(), 1);
}

#[test]
fn reports_http_failures_with_redaction() {
    let transport = Arc::new(FixtureTransport::new(TransportLimits::default()));
    transport.push(Response::text(
        401,
        r#"{"error":{"message":"bad key fake-key"}}"#,
    ));
    let mut service = adapter(Arc::clone(&transport));
    let mut collector = Collector::default();
    let error = service
        .run(request(Vec::new(), 1), &mut collector)
        .unwrap_err();
    assert!(error.message.contains("401"));
    assert!(!error.message.contains("fake-key"), "{}", error.message);
}

#[test]
fn reports_transport_failures() {
    let transport = Arc::new(FixtureTransport::new(TransportLimits::default()));
    for _ in 0..3 {
        transport.push_error(TransportErrorKind::Timeout, "late");
    }
    let mut service = adapter(Arc::clone(&transport));
    let mut collector = Collector::default();
    let error = service
        .run(request(Vec::new(), 1), &mut collector)
        .unwrap_err();
    assert!(error.message.contains("timed out"));
}

#[test]
fn rejects_a_malformed_body() {
    let transport = Arc::new(FixtureTransport::new(TransportLimits::default()));
    transport.push(Response::text(200, "not json"));
    let mut service = adapter(Arc::clone(&transport));
    let mut collector = Collector::default();
    let error = service
        .run(request(Vec::new(), 1), &mut collector)
        .unwrap_err();
    assert!(error.message.contains("not valid JSON"));
}

#[test]
fn fetches_the_model_catalog_for_its_own_service_only() {
    let transport = Arc::new(FixtureTransport::new(TransportLimits::default()));
    transport.push(Response::text(
        200,
        r#"{"object":"list","data":[{"id":"deepseek-chat","object":"model"}]}"#,
    ));
    let service = adapter(Arc::clone(&transport));
    let document = service.fetch(ServiceId::DeepSeek).unwrap();
    let entries =
        koru::provider::catalog::parse_service_catalog(ServiceId::DeepSeek, &document, 1, "test")
            .unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].id, "deepseek-chat");
    assert_eq!(
        service.fetch(ServiceId::OpenCode).unwrap_err().code(),
        ErrorCode::UnsupportedCapability
    );
    let sent = transport.requests();
    assert_eq!(sent[0].url, "https://api.deepseek.com/models");
    assert_eq!(sent[0].header("authorization"), Some("Bearer fake-key"));
}
