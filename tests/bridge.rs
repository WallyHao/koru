//! Single-VM bridge gate: suspend/resume, serial tool dispatch, bounds, cancel.
use koru::{
    ai::{
        AiService, ServiceError,
        fake::{FakeAiService, FakeRound},
    },
    error::{ErrorCode, KoruError, Result},
    json::JsonValue,
    lua::{CANCELLATION_LATENCY_TARGET, LoadedCommand},
    runtime::{ExecutionContext, Limits},
    source::{SourceBundle, SourceLimits},
};
use std::{
    fs,
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};
use tempfile::TempDir;

struct Fixture {
    _dir: TempDir,
    bundle: SourceBundle,
    context: ExecutionContext,
}
impl Fixture {
    fn new(entry: &str, limits: Limits) -> Self {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("lib")).unwrap();
        fs::write(dir.path().join("demo.lua"), entry).unwrap();
        let bundle = SourceBundle::capture(dir.path(), "demo", SourceLimits::default()).unwrap();
        let context =
            ExecutionContext::new("demo", bundle.digest(), limits, Instant::now()).unwrap();
        Self {
            _dir: dir,
            bundle,
            context,
        }
    }
    fn run(&self, service: Box<dyn AiService>, args: &JsonValue) -> Result<JsonValue> {
        let loaded = LoadedCommand::load(&self.bundle, &self.context).unwrap();
        loaded.run(service, args)
    }
}

fn object(pairs: Vec<(&str, JsonValue)>) -> JsonValue {
    JsonValue::Object(pairs.into_iter().map(|(k, v)| (k.to_owned(), v)).collect())
}
fn empty() -> JsonValue {
    object(Vec::new())
}
fn field<'a>(value: &'a JsonValue, key: &str) -> &'a JsonValue {
    match value {
        JsonValue::Object(entries) => entries.get(key).unwrap(),
        other => panic!("expected object, found {}", other.kind()),
    }
}

const ASK: &str = r#"
return {
  api_version = 1,
  description = "demo",
  run = function(koru, args)
    local r = koru.ai.ask("hello")
    return { text = r.text, model = r.model, reason = r.finish_reason, name = args.name }
  end,
}
"#;

#[test]
fn workflow_suspends_and_receives_the_ai_result() {
    let fixture = Fixture::new(ASK, Limits::default());
    let args = object(vec![("name", JsonValue::String("ada".to_owned()))]);
    let result = fixture
        .run(
            Box::new(FakeAiService::answer("world").model("fake-1")),
            &args,
        )
        .unwrap();
    assert_eq!(
        field(&result, "text"),
        &JsonValue::String("world".to_owned())
    );
    assert_eq!(
        field(&result, "model"),
        &JsonValue::String("fake-1".to_owned())
    );
    assert_eq!(
        field(&result, "reason"),
        &JsonValue::String("stop".to_owned())
    );
    assert_eq!(field(&result, "name"), &JsonValue::String("ada".to_owned()));
}

const TOOLS: &str = r#"
local log = {}
return {
  api_version = 1,
  description = "demo",
  tools = {
    {
      name = "add",
      description = "adds two integers",
      parameters = { type = "object" },
      run = function(args)
        log[#log + 1] = args.a + args.b
        return { sum = args.a + args.b }
      end,
    },
  },
  run = function(koru, args)
    local r = koru.ai.run({ prompt = "go", tools = { "add" }, max_turns = 4 })
    return { text = r.text, calls = log }
  end,
}
"#;

#[test]
fn serial_tool_calls_run_in_stable_order() {
    let fixture = Fixture::new(TOOLS, Limits::default());
    let service = FakeAiService::with_rounds(vec![
        FakeRound::calls(vec![(
            "add".to_owned(),
            object(vec![
                ("a", JsonValue::Integer(1)),
                ("b", JsonValue::Integer(2)),
            ]),
        )]),
        FakeRound::calls(vec![(
            "add".to_owned(),
            object(vec![
                ("a", JsonValue::Integer(3)),
                ("b", JsonValue::Integer(4)),
            ]),
        )]),
    ]);
    let result = fixture.run(Box::new(service), &empty()).unwrap();
    assert_eq!(
        field(&result, "calls"),
        &JsonValue::Array(vec![JsonValue::Integer(3), JsonValue::Integer(7)])
    );
}

#[test]
fn one_workflow_can_make_several_ai_calls() {
    let entry = r#"
return {
  api_version = 1,
  description = "demo",
  run = function(koru, args)
    local first = koru.ai.ask("one")
    local second = koru.ai.ask("two")
    return { first = first.text, second = second.text }
  end,
}
"#;
    let fixture = Fixture::new(entry, Limits::default());
    let result = fixture
        .run(Box::new(FakeAiService::answer("ok")), &empty())
        .unwrap();
    assert_eq!(field(&result, "first"), &JsonValue::String("ok".to_owned()));
    assert_eq!(
        field(&result, "second"),
        &JsonValue::String("ok".to_owned())
    );
}

#[test]
fn nested_ai_calls_from_a_tool_are_rejected() {
    let entry = r#"
return {
  api_version = 1,
  description = "demo",
  tools = {
    {
      name = "bad",
      description = "tries to nest",
      parameters = { type = "object" },
      run = function(args)
        local _ = koru.ai.run({ prompt = "nested" })
        return {}
      end,
    },
  },
  run = function(koru, args)
    local _ = koru.ai.run({ prompt = "go", tools = { "bad" }, max_turns = 2 })
    return {}
  end,
}
"#;
    let fixture = Fixture::new(entry, Limits::default());
    let service =
        FakeAiService::with_rounds(vec![FakeRound::calls(vec![("bad".to_owned(), empty())])]);
    assert_eq!(
        fixture.run(Box::new(service), &empty()).unwrap_err().code(),
        ErrorCode::ToolFailure
    );
}

#[test]
fn unknown_tool_selection_fails_before_any_request() {
    let entry = r#"
return {
  api_version = 1,
  description = "demo",
  run = function(koru, args)
    local _ = koru.ai.run({ prompt = "go", tools = { "missing" } })
    return {}
  end,
}
"#;
    let fixture = Fixture::new(entry, Limits::default());
    assert_eq!(
        fixture
            .run(Box::new(FakeAiService::answer("x")), &empty())
            .unwrap_err()
            .code(),
        ErrorCode::Validation
    );
}

#[test]
fn tool_budget_is_enforced_across_the_agent_run() {
    let fixture = Fixture::new(
        TOOLS,
        Limits {
            tool_calls: 1,
            ..Limits::default()
        },
    );
    let service = FakeAiService::with_rounds(vec![
        FakeRound::calls(vec![(
            "add".to_owned(),
            object(vec![
                ("a", JsonValue::Integer(1)),
                ("b", JsonValue::Integer(1)),
            ]),
        )]),
        FakeRound::calls(vec![(
            "add".to_owned(),
            object(vec![
                ("a", JsonValue::Integer(2)),
                ("b", JsonValue::Integer(2)),
            ]),
        )]),
    ]);
    assert_eq!(
        fixture.run(Box::new(service), &empty()).unwrap_err().code(),
        ErrorCode::BudgetExhausted
    );
}

#[test]
fn model_turn_limit_is_reported() {
    let entry = r#"
return {
  api_version = 1,
  description = "demo",
  tools = {
    { name = "add", description = "adds", parameters = { type = "object" }, run = function(args) return {} end },
  },
  run = function(koru, args)
    local r = koru.ai.run({ prompt = "go", tools = { "add" }, max_turns = 1 })
    return { reason = r.finish_reason }
  end,
}
"#;
    let fixture = Fixture::new(entry, Limits::default());
    let service = FakeAiService::with_rounds(vec![
        FakeRound::calls(vec![("add".to_owned(), empty())]),
        FakeRound::calls(vec![("add".to_owned(), empty())]),
    ]);
    let result = fixture.run(Box::new(service), &empty()).unwrap();
    assert_eq!(
        field(&result, "reason"),
        &JsonValue::String("length".to_owned())
    );
}

#[test]
fn cancellation_while_waiting_stops_the_run() {
    let fixture = Fixture::new(ASK, Limits::default());
    let cancel = fixture.context.clone();
    let canceller = thread::spawn(move || {
        thread::sleep(std::time::Duration::from_millis(20));
        cancel.cancel().unwrap();
    });
    let service = FakeAiService::answer("late").delay(std::time::Duration::from_millis(200));
    let error = fixture
        .run(Box::new(service), &object(vec![("name", JsonValue::Null)]))
        .unwrap_err();
    canceller.join().unwrap();
    assert_eq!(error.code(), ErrorCode::Cancelled);
}

#[test]
fn provider_failures_are_reported_as_provider_failures() {
    let fixture = Fixture::new(ASK, Limits::default());
    let service = FakeAiService::failing(ServiceError::provider("upstream down"));
    let error = fixture
        .run(Box::new(service), &object(vec![("name", JsonValue::Null)]))
        .unwrap_err();
    assert_eq!(error.code(), ErrorCode::ProviderFailure);
    assert!(error.message().contains("upstream down") || error.message().contains("provider"));
}

#[test]
fn workflow_return_values_convert_to_json() {
    let entry = r#"
return {
  api_version = 1,
  description = "demo",
  run = function(koru, args)
    return { nested = { true, koru.json.null }, empty = koru.json.array({}) }
  end,
}
"#;
    let fixture = Fixture::new(entry, Limits::default());
    let result = fixture
        .run(Box::new(FakeAiService::answer("unused")), &empty())
        .unwrap();
    assert_eq!(
        field(&result, "nested"),
        &JsonValue::Array(vec![JsonValue::Bool(true), JsonValue::Null])
    );
    assert_eq!(field(&result, "empty"), &JsonValue::Array(Vec::new()));
}

#[test]
fn tool_arguments_are_validated_before_dispatch() {
    let entry = r#"
return {
  api_version = 1,
  description = "demo",
  tools = {
    {
      name = "add",
      description = "adds two integers",
      parameters = {
        type = "object",
        properties = { a = { type = "integer" }, b = { type = "integer" } },
        required = { "a", "b" },
      },
      run = function(args) error("CALLBACK_RAN") end,
    },
  },
  run = function(koru, args)
    local _ = koru.ai.run({ prompt = "go", tools = { "add" } })
    return {}
  end,
}
"#;
    let fixture = Fixture::new(entry, Limits::default());
    let service = FakeAiService::with_rounds(vec![FakeRound::calls(vec![(
        "add".to_owned(),
        object(vec![
            ("a", JsonValue::String("nope".to_owned())),
            ("b", JsonValue::Integer(1)),
        ]),
    )])]);
    let error = fixture.run(Box::new(service), &empty()).unwrap_err();
    assert_eq!(error.code(), ErrorCode::Validation);
    assert!(!error.message().contains("CALLBACK_RAN"));
    assert!(error.message().contains("a"));
}

#[test]
fn declared_tool_results_are_validated() {
    let entry = r#"
return {
  api_version = 1,
  description = "demo",
  tools = {
    {
      name = "total",
      description = "sums",
      parameters = { type = "object" },
      result = {
        type = "object",
        properties = { total = { type = "integer" } },
        required = { "total" },
      },
      run = function(args) return { total = "not-an-integer" } end,
    },
  },
  run = function(koru, args)
    local _ = koru.ai.run({ prompt = "go", tools = { "total" } })
    return {}
  end,
}
"#;
    let fixture = Fixture::new(entry, Limits::default());
    let service =
        FakeAiService::with_rounds(vec![FakeRound::calls(vec![("total".to_owned(), empty())])]);
    let error = fixture.run(Box::new(service), &empty()).unwrap_err();
    assert_eq!(error.code(), ErrorCode::Validation);
    assert!(error.message().contains("result"));
}

#[test]
fn valid_tool_contracts_dispatch() {
    let entry = r#"
return {
  api_version = 1,
  description = "demo",
  tools = {
    {
      name = "next",
      description = "increments",
      parameters = {
        type = "object",
        properties = { value = { type = "integer" } },
        required = { "value" },
      },
      result = {
        type = "object",
        properties = { next = { type = "integer" } },
        required = { "next" },
      },
      run = function(args) return { next = args.value + 1 } end,
    },
  },
  run = function(koru, args)
    local r = koru.ai.run({ prompt = "go", tools = { "next" } })
    return { text = r.text }
  end,
}
"#;
    let fixture = Fixture::new(entry, Limits::default());
    let service = FakeAiService::with_rounds(vec![FakeRound::calls(vec![(
        "next".to_owned(),
        object(vec![("value", JsonValue::Integer(41))]),
    )])]);
    let result = fixture.run(Box::new(service), &empty()).unwrap();
    assert_eq!(
        field(&result, "text"),
        &JsonValue::String("done".to_owned())
    );
}

#[test]
fn greedy_tool_calls_are_bounded_by_the_budget() {
    let entry = r#"
return {
  api_version = 1,
  description = "demo",
  tools = {
    { name = "noop", description = "does nothing", parameters = { type = "object" }, run = function(args) return {} end },
  },
  run = function(koru, args)
    local _ = koru.ai.run({ prompt = "go", tools = { "noop" }, max_turns = 8 })
    return {}
  end,
}
"#;
    let fixture = Fixture::new(
        entry,
        Limits {
            tool_calls: 64,
            ..Limits::default()
        },
    );
    let calls = (0..200)
        .map(|_| ("noop".to_owned(), empty()))
        .collect::<Vec<_>>();
    let service = FakeAiService::with_rounds(vec![FakeRound::calls(calls)]);
    let error = fixture.run(Box::new(service), &empty()).unwrap_err();
    assert_eq!(error.code(), ErrorCode::BudgetExhausted);
    assert_eq!(fixture.context.usage().unwrap().tool_calls, 64);
}

#[test]
fn uncooperative_service_does_not_block_shutdown() {
    let fixture = Fixture::new(
        ASK,
        Limits {
            wall_time: Duration::from_millis(100),
            ..Limits::default()
        },
    );
    let service = FakeAiService::answer("late").delay(Duration::from_secs(5));
    let start = Instant::now();
    let error = fixture.run(Box::new(service), &empty()).unwrap_err();
    assert_eq!(error.code(), ErrorCode::Timeout);
    assert!(start.elapsed() < Duration::from_secs(2));
}

#[test]
fn cancellation_latency_is_bounded() {
    let fixture = Fixture::new(ASK, Limits::default());
    let cancel = fixture.context.clone();
    let cancelled_at = Arc::new(Mutex::new(None::<Instant>));
    let slot = Arc::clone(&cancelled_at);
    let canceller = thread::spawn(move || {
        thread::sleep(Duration::from_millis(20));
        cancel.cancel().unwrap();
        *slot.lock().unwrap() = Some(Instant::now());
    });
    let service = FakeAiService::answer("late").delay(Duration::from_millis(200));
    let error = fixture
        .run(Box::new(service), &object(vec![("name", JsonValue::Null)]))
        .unwrap_err();
    let returned = Instant::now();
    canceller.join().unwrap();
    assert_eq!(error.code(), ErrorCode::Cancelled);
    let at = cancelled_at.lock().unwrap().unwrap();
    assert!(returned.duration_since(at) < CANCELLATION_LATENCY_TARGET * 4);
}

#[test]
fn load_failures_do_not_start_a_run() {
    let fixture = Fixture::new(
        r#"return { api_version = 1, description = "demo" }"#,
        Limits::default(),
    );
    let error: KoruError = match LoadedCommand::load(&fixture.bundle, &fixture.context) {
        Ok(_) => panic!("expected load to fail"),
        Err(error) => error,
    };
    assert_eq!(error.code(), ErrorCode::Validation);
}
