//! Retry classification and budget integration.
use koru::{
    ai::{
        AiRequest, AiResult, AiService, FinishReason, ServiceCapabilities, ServiceError,
        ServiceEvents, ToolCall, ToolResult, Usage,
    },
    error::{ErrorCode, Result},
    json::JsonValue,
    lua::LoadedCommand,
    provider::DeepSeekAdapter,
    runtime::{ExecutionContext, Limits},
    source::{SourceBundle, SourceLimits},
    transport::{FixtureTransport, Response, TransportErrorKind, TransportLimits},
};
use std::{fs, sync::Arc, time::Instant};
use tempfile::TempDir;

struct Collector {
    retries: usize,
    deny: bool,
}
impl ServiceEvents for Collector {
    fn call_tool(&mut self, _call: ToolCall) -> std::result::Result<ToolResult, ServiceError> {
        Ok(ToolResult {
            content: JsonValue::Null,
        })
    }
    fn reserve_retry(&mut self) -> std::result::Result<(), ServiceError> {
        if self.deny {
            return Err(ServiceError::provider("retry budget exhausted"));
        }
        self.retries += 1;
        Ok(())
    }
}

fn request() -> AiRequest {
    AiRequest {
        prompt: "hello".to_owned(),
        tools: Vec::new(),
        max_turns: 1,
    }
}
fn adapter(transport: Arc<FixtureTransport>) -> DeepSeekAdapter {
    DeepSeekAdapter::new(transport, "fake-key", "deepseek-chat", None)
}
fn text_response() -> Response {
    Response::text(
        200,
        r#"{"choices":[{"message":{"content":"ok"},"finish_reason":"stop"}]}"#,
    )
}

#[test]
fn retries_a_transient_transport_failure() {
    let transport = Arc::new(FixtureTransport::new(TransportLimits::default()));
    transport.push_error(TransportErrorKind::Timeout, "late");
    transport.push(text_response());
    let mut service = adapter(Arc::clone(&transport));
    let mut collector = Collector {
        retries: 0,
        deny: false,
    };
    let result = service.run(request(), &mut collector).unwrap();
    assert_eq!(result.text, "ok");
    assert_eq!(collector.retries, 1);
    assert_eq!(transport.requests().len(), 2);
}

#[test]
fn retries_a_retryable_status() {
    let transport = Arc::new(FixtureTransport::new(TransportLimits::default()));
    let mut busy = Response::text(503, "unavailable");
    busy.headers
        .push(("Retry-After".to_owned(), "0".to_owned()));
    transport.push(busy);
    transport.push(text_response());
    let mut service = adapter(Arc::clone(&transport));
    let mut collector = Collector {
        retries: 0,
        deny: false,
    };
    service.run(request(), &mut collector).unwrap();
    assert_eq!(collector.retries, 1);
    assert_eq!(transport.requests().len(), 2);
}

#[test]
fn does_not_retry_a_client_error() {
    let transport = Arc::new(FixtureTransport::new(TransportLimits::default()));
    transport.push(Response::text(400, "bad request"));
    transport.push(text_response());
    let mut service = adapter(Arc::clone(&transport));
    let mut collector = Collector {
        retries: 0,
        deny: false,
    };
    let error = service.run(request(), &mut collector).unwrap_err();
    assert!(error.message.contains("400"));
    assert_eq!(collector.retries, 0);
    assert_eq!(transport.requests().len(), 1);
}

#[test]
fn stops_after_the_attempt_cap() {
    let transport = Arc::new(FixtureTransport::new(TransportLimits::default()));
    transport.push_error(TransportErrorKind::Connect, "down");
    transport.push_error(TransportErrorKind::Connect, "down");
    transport.push_error(TransportErrorKind::Connect, "down");
    transport.push(text_response());
    let mut service = adapter(Arc::clone(&transport));
    let mut collector = Collector {
        retries: 0,
        deny: false,
    };
    let error = service.run(request(), &mut collector).unwrap_err();
    assert!(error.message.contains("cannot reach"));
    assert_eq!(collector.retries, 2);
    assert_eq!(transport.requests().len(), 3);
}

#[test]
fn a_denied_retry_budget_surfaces() {
    let transport = Arc::new(FixtureTransport::new(TransportLimits::default()));
    transport.push_error(TransportErrorKind::Timeout, "late");
    transport.push(text_response());
    let mut service = adapter(Arc::clone(&transport));
    let mut collector = Collector {
        retries: 0,
        deny: true,
    };
    let error = service.run(request(), &mut collector).unwrap_err();
    assert!(error.message.contains("retry budget"));
    assert_eq!(transport.requests().len(), 1);
}

struct ChargingService;
impl AiService for ChargingService {
    fn capabilities(&self) -> ServiceCapabilities {
        ServiceCapabilities::tool_capable("charging")
    }
    fn run(
        &mut self,
        _request: AiRequest,
        events: &mut dyn ServiceEvents,
    ) -> std::result::Result<AiResult, ServiceError> {
        events.reserve_retry()?;
        Ok(AiResult {
            text: "ok".to_owned(),
            finish_reason: FinishReason::Stop,
            model: "m".to_owned(),
            usage: Usage::default(),
            request_id: None,
        })
    }
}

struct Fixture {
    _dir: TempDir,
    bundle: SourceBundle,
    context: ExecutionContext,
}
impl Fixture {
    fn new(limits: Limits) -> Self {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("lib")).unwrap();
        fs::write(
            dir.path().join("demo.lua"),
            r#"
return {
  api_version = 1,
  description = "demo",
  run = function(koru, args)
    local r = koru.ai.ask("x")
    return { text = r.text }
  end,
}
"#,
        )
        .unwrap();
        let bundle = SourceBundle::capture(dir.path(), "demo", SourceLimits::default()).unwrap();
        let context =
            ExecutionContext::new("demo", bundle.digest(), limits, Instant::now()).unwrap();
        Self {
            _dir: dir,
            bundle,
            context,
        }
    }
    fn run(&self, service: Box<dyn AiService>) -> Result<JsonValue> {
        let loaded = LoadedCommand::load(&self.bundle, &self.context).unwrap();
        loaded.run(service, &JsonValue::Object(Default::default()))
    }
}

#[test]
fn owner_charges_each_retry_to_the_model_request_budget() {
    let fixture = Fixture::new(Limits::default());
    let result = fixture.run(Box::new(ChargingService)).unwrap();
    let JsonValue::Object(entries) = result else {
        panic!("expected object");
    };
    assert_eq!(
        entries.get("text"),
        Some(&JsonValue::String("ok".to_owned()))
    );
    assert_eq!(fixture.context.usage().unwrap().model_requests, 2);
}

#[test]
fn owner_rejects_a_retry_when_the_budget_is_spent() {
    let fixture = Fixture::new(Limits {
        model_requests: 1,
        ..Limits::default()
    });
    let error = fixture.run(Box::new(ChargingService)).unwrap_err();
    assert_eq!(error.code(), ErrorCode::BudgetExhausted);
    assert_eq!(fixture.context.usage().unwrap().model_requests, 1);
}
