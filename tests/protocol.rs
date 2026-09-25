//! Chat Completions protocol mapping: request shapes and response parsing.
use koru::{
    ai::{FinishReason, ToolCall, ToolResult, ToolSpec},
    error::ErrorCode,
    json::{JsonLimits, JsonValue, emit, parse},
    provider::protocol::{Conversation, build_request, parse_response},
    schema::JsonSchema,
};

fn json(text: &str) -> JsonValue {
    parse(text.as_bytes(), &JsonLimits::default()).unwrap()
}
fn spec() -> ToolSpec {
    let document =
        json(r#"{"type":"object","properties":{"a":{"type":"integer"}},"required":["a"]}"#);
    ToolSpec {
        name: "add".to_owned(),
        description: "adds".to_owned(),
        parameters: JsonSchema::compile(&document, &JsonLimits::default()).unwrap(),
    }
}
fn call(id: &str, name: &str, arguments: JsonValue) -> ToolCall {
    ToolCall {
        id: id.to_owned(),
        name: name.to_owned(),
        arguments,
    }
}

#[test]
fn builds_a_tool_request() {
    let conversation = Conversation::new("hello");
    let request = build_request("deepseek-chat", &conversation, &[spec()], None);
    let expected = json(
        r#"{
          "model": "deepseek-chat",
          "messages": [{"role": "user", "content": "hello"}],
          "stream": false,
          "tools": [{
            "type": "function",
            "function": {
              "name": "add",
              "description": "adds",
              "parameters": {
                "type": "object",
                "properties": {"a": {"type": "integer"}},
                "required": ["a"]
              }
            }
          }]
        }"#,
    );
    assert_eq!(emit(&request), emit(&expected));
}

#[test]
fn omits_tools_when_none_are_offered() {
    let conversation = Conversation::new("hello");
    let request = build_request("m", &conversation, &[], None);
    assert_eq!(
        emit(&request),
        emit(&json(
            r#"{"model":"m","messages":[{"role":"user","content":"hello"}],"stream":false}"#
        ))
    );
}

#[test]
fn includes_the_mapped_effort_when_present() {
    let conversation = Conversation::new("hello");
    let request = build_request("m", &conversation, &[], Some("high"));
    assert_eq!(
        request,
        json(
            r#"{"model":"m","messages":[{"role":"user","content":"hello"}],"stream":false,"reasoning_effort":"high"}"#
        )
    );
}

#[test]
fn appends_assistant_tool_calls_and_tool_results() {
    let mut conversation = Conversation::new("go");
    conversation.push_assistant(None, &[call("call-1", "add", json(r#"{"a":1}"#))]);
    conversation.push_tool_result(
        "call-1",
        &ToolResult {
            content: json(r#"{"sum":1}"#),
        },
    );
    assert_eq!(conversation.messages().len(), 3);
    assert_eq!(
        conversation.messages()[1],
        json(
            r#"{"role":"assistant","content":null,"tool_calls":[{"id":"call-1","type":"function","function":{"name":"add","arguments":"{\"a\":1}"}}]}"#
        )
    );
    assert_eq!(
        conversation.messages()[2],
        json(r#"{"role":"tool","tool_call_id":"call-1","content":"{\"sum\":1}"}"#)
    );
}

#[test]
fn parses_a_tool_call_response() {
    let response = parse_response(&json(
        r#"{
          "id": "req-1",
          "choices": [{
            "message": {
              "content": null,
              "tool_calls": [{
                "id": "call-9",
                "type": "function",
                "function": {"name": "add", "arguments": "{\"a\":1,\"b\":2}"}
              }]
            },
            "finish_reason": "tool_calls"
          }],
          "usage": {"prompt_tokens": 10, "completion_tokens": 4}
        }"#,
    ))
    .unwrap();
    assert_eq!(response.text, "");
    assert_eq!(response.finish, FinishReason::ToolCalls);
    assert_eq!(response.request_id.as_deref(), Some("req-1"));
    assert_eq!(response.usage.input_tokens, Some(10));
    assert_eq!(response.usage.output_tokens, Some(4));
    assert_eq!(response.calls.len(), 1);
    assert_eq!(response.calls[0].id, "call-9");
    assert_eq!(response.calls[0].name, "add");
    assert_eq!(response.calls[0].arguments, json(r#"{"a":1,"b":2}"#));
}

#[test]
fn parses_text_and_maps_finish_reasons() {
    let response = parse_response(&json(
        r#"{"choices":[{"message":{"content":"done"},"finish_reason":"stop"}]}"#,
    ))
    .unwrap();
    assert_eq!(response.text, "done");
    assert_eq!(response.finish, FinishReason::Stop);
    assert!(response.calls.is_empty());
    assert_eq!(response.usage.input_tokens, None);

    for (reason, expected) in [
        ("length", FinishReason::Length),
        ("content_filter", FinishReason::ContentFilter),
        ("weird", FinishReason::Other("weird".to_owned())),
    ] {
        let document = json(&format!(
            r#"{{"choices":[{{"message":{{"content":"x"}},"finish_reason":"{reason}"}}]}}"#
        ));
        assert_eq!(parse_response(&document).unwrap().finish, expected);
    }
}

#[test]
fn rejects_malformed_responses() {
    for text in [
        r#"{}"#,
        r#"{"choices":[]}"#,
        r#"{"choices":[{}]}"#,
        r#"{"choices":[{"message":{}}]}"#,
        r#"{"choices":[{"message":{"content":1}}]}"#,
        r#"{"choices":[{"message":{"content":"x","tool_calls":1}}]}"#,
        r#"{"choices":[{"message":{"content":"x","tool_calls":[{}]}}]}"#,
        r#"{"choices":[{"message":{"content":"x","tool_calls":[{"id":"c","function":{"name":"n","arguments":"not json"}}]}}]}"#,
        r#"{"choices":[{"message":{"content":"x","tool_calls":[{"id":"c","function":{"name":"n","arguments":"[1]"}}]}}]}"#,
        r#"{"choices":[{"message":{"content":"x"}}],"usage":{"prompt_tokens":"10"}}"#,
        r#"{"choices":[{"message":{"content":"x","tool_calls":[{"id":"c","function":{"name":"n","arguments":"{}"}},{"id":"c","function":{"name":"n","arguments":"{}"}}]}}]}"#,
    ] {
        let error = parse_response(&json(text)).unwrap_err();
        assert_eq!(error.code(), ErrorCode::ProviderFailure, "text: {text}");
    }
}
