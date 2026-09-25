//! A captured Lua command reaches a real provider adapter through the VM bridge.
use koru::{
    json::JsonValue,
    lua::LoadedCommand,
    provider::DeepSeekAdapter,
    runtime::{ExecutionContext, Limits},
    source::{SourceBundle, SourceLimits},
    transport::{FixtureTransport, Response, TransportLimits},
};
use std::{fs, sync::Arc, time::Instant};

#[test]
fn lua_ask_uses_selected_adapter_and_returns_its_answer() {
    let root = tempfile::tempdir().unwrap();
    fs::write(
        root.path().join("ask.lua"),
        r#"
return {
  api_version = 1,
  description = "Ask a model",
  arguments = {{ name = "question", type = "string", required = true }},
  run = function(koru, args)
    local answer = koru.ai.ask(args.question)
    return answer.text
  end,
}
"#,
    )
    .unwrap();
    let bundle = SourceBundle::capture(root.path(), "ask", SourceLimits::default()).unwrap();
    let context =
        ExecutionContext::new("ask", bundle.digest(), Limits::default(), Instant::now()).unwrap();
    let loaded = LoadedCommand::load(&bundle, &context).unwrap();
    let args = loaded
        .declaration()
        .parse_args(&["hello".to_owned()])
        .unwrap();
    let transport = Arc::new(FixtureTransport::new(TransportLimits::default()));
    transport.push(Response::text(
        200,
        r#"{"id":"req-1","choices":[{"message":{"content":"world"},"finish_reason":"stop"}]}"#,
    ));
    let adapter = DeepSeekAdapter::new(transport.clone(), "fixture-key", "deepseek-chat", None);
    let result = loaded.run(Box::new(adapter), &args).unwrap();
    assert_eq!(result, JsonValue::String("world".to_owned()));
    let requests = transport.requests();
    assert_eq!(requests.len(), 1);
    assert!(String::from_utf8_lossy(&requests[0].body).contains("hello"));
}
