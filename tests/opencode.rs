//! OpenCode Zen/Go adapters: endpoints, sessions, capabilities, and catalogs.
use koru::{
    ai::{AiRequest, AiService, ServiceError, ServiceEvents, ToolCall, ToolResult},
    error::ErrorCode,
    json::JsonValue,
    provider::{
        OpenCodeAdapter, OpenCodeSurface, ServiceId, catalog::CatalogSource,
        opencode::SESSION_HEADER,
    },
    transport::{FixtureTransport, Response, TransportErrorKind, TransportLimits},
};
use std::sync::Arc;

struct Collector {
    calls: Vec<ToolCall>,
}
impl ServiceEvents for Collector {
    fn call_tool(&mut self, call: ToolCall) -> Result<ToolResult, ServiceError> {
        self.calls.push(call);
        Ok(ToolResult {
            content: JsonValue::Null,
        })
    }
}

fn request() -> AiRequest {
    AiRequest {
        prompt: "hello".to_owned(),
        tools: Vec::new(),
        max_turns: 1,
    }
}

fn adapter(surface: OpenCodeSurface, transport: Arc<FixtureTransport>) -> OpenCodeAdapter {
    OpenCodeAdapter::new(surface, transport, "oc-key", "model-x", None)
}

#[test]
fn separates_zen_and_go_endpoints_and_sessions() {
    let go_transport = Arc::new(FixtureTransport::new(TransportLimits::default()));
    go_transport.push(Response::text(
        200,
        r#"{"choices":[{"message":{"content":"hi"},"finish_reason":"stop"}]}"#,
    ));
    let mut go = adapter(OpenCodeSurface::Go, Arc::clone(&go_transport));
    go.run(request(), &mut Collector { calls: Vec::new() })
        .unwrap();
    let go_sent = go_transport.requests();
    assert_eq!(
        go_sent[0].url,
        "https://opencode.ai/zen/go/v1/chat/completions"
    );
    let session = go_sent[0]
        .header(SESSION_HEADER)
        .expect("go session header");
    assert!(session.starts_with("koru-"));

    let zen_transport = Arc::new(FixtureTransport::new(TransportLimits::default()));
    zen_transport.push(Response::text(
        200,
        r#"{"choices":[{"message":{"content":"hi"},"finish_reason":"stop"}]}"#,
    ));
    let mut zen = adapter(OpenCodeSurface::Zen, Arc::clone(&zen_transport));
    zen.run(request(), &mut Collector { calls: Vec::new() })
        .unwrap();
    let zen_sent = zen_transport.requests();
    assert_eq!(
        zen_sent[0].url,
        "https://opencode.ai/zen/v1/chat/completions"
    );
    assert_eq!(zen_sent[0].header(SESSION_HEADER), None);
}

#[test]
fn session_is_stable_across_a_tool_loop() {
    let transport = Arc::new(FixtureTransport::new(TransportLimits::default()));
    transport.push(Response::text(
        200,
        r#"{"choices":[{"message":{"content":null,"tool_calls":[{"id":"call-1","type":"function","function":{"name":"add","arguments":"{}"}}]},"finish_reason":"tool_calls"}]}"#,
    ));
    transport.push(Response::text(
        200,
        r#"{"choices":[{"message":{"content":"done"},"finish_reason":"stop"}]}"#,
    ));
    let mut service = adapter(OpenCodeSurface::Go, Arc::clone(&transport));
    let mut collector = Collector { calls: Vec::new() };
    let mut request = request();
    request.max_turns = 4;
    service.run(request, &mut collector).unwrap();
    let sent = transport.requests();
    assert_eq!(sent.len(), 2);
    let first = sent[0].header(SESSION_HEADER).unwrap();
    let second = sent[1].header(SESSION_HEADER).unwrap();
    assert_eq!(first, second);
}

#[test]
fn capabilities_match_the_surface() {
    let transport = Arc::new(FixtureTransport::new(TransportLimits::default()));
    let go = adapter(OpenCodeSurface::Go, Arc::clone(&transport));
    let capabilities = go.capabilities();
    assert_eq!(capabilities.provider, "opencode-go");
    assert!(capabilities.sessions);
    assert_eq!(capabilities.variants, vec!["high".to_owned()]);

    let zen = adapter(OpenCodeSurface::Zen, transport);
    let capabilities = zen.capabilities();
    assert_eq!(capabilities.provider, "opencode");
    assert!(!capabilities.sessions);
    assert!(capabilities.variants.is_empty());
}

#[test]
fn catalog_fetch_is_service_scoped() {
    let transport = Arc::new(FixtureTransport::new(TransportLimits::default()));
    transport.push(Response::text(200, r#"{"data":[]}"#));
    transport.push(Response::text(200, r#"{"data":[]}"#));
    let go = adapter(OpenCodeSurface::Go, Arc::clone(&transport));
    assert!(go.fetch(ServiceId::OpenCodeGo).is_ok());
    assert_eq!(
        go.fetch(ServiceId::OpenCode).unwrap_err().code(),
        ErrorCode::UnsupportedCapability
    );
    let sent = transport.requests();
    assert_eq!(sent[0].url, "https://opencode.ai/zen/go/v1/models");
    assert!(sent[0].header(SESSION_HEADER).is_some());
}

#[test]
fn redacts_credentials_in_errors() {
    let transport = Arc::new(FixtureTransport::new(TransportLimits::default()));
    transport.push(Response::text(403, "forbidden for oc-key"));
    let mut go = adapter(OpenCodeSurface::Go, Arc::clone(&transport));
    let error = go
        .run(request(), &mut Collector { calls: Vec::new() })
        .unwrap_err();
    assert!(error.message.contains("403"));
    assert!(!error.message.contains("oc-key"), "{}", error.message);
}

#[test]
fn reports_transport_failures() {
    let transport = Arc::new(FixtureTransport::new(TransportLimits::default()));
    transport.push_error(TransportErrorKind::Connect, "down");
    let mut go = adapter(OpenCodeSurface::Go, Arc::clone(&transport));
    let error = go
        .run(request(), &mut Collector { calls: Vec::new() })
        .unwrap_err();
    assert!(error.message.contains("cannot reach"));
}
