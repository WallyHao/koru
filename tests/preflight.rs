//! Capability preflight: unsupported requests fail before any model request.
use koru::{
    ai::{
        AiService, ServiceCapabilities,
        fake::{FakeAiService, FakeRound},
    },
    error::{ErrorCode, Result},
    json::JsonValue,
    lua::LoadedCommand,
    runtime::{ExecutionContext, Limits},
    source::{SourceBundle, SourceLimits},
};
use std::{fs, time::Instant};
use tempfile::TempDir;

struct Fixture {
    _dir: TempDir,
    bundle: SourceBundle,
    context: ExecutionContext,
}
impl Fixture {
    fn new(entry: &str) -> Self {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("lib")).unwrap();
        fs::write(dir.path().join("demo.lua"), entry).unwrap();
        let bundle = SourceBundle::capture(dir.path(), "demo", SourceLimits::default()).unwrap();
        let context =
            ExecutionContext::new("demo", bundle.digest(), Limits::default(), Instant::now())
                .unwrap();
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

const ENTRY: &str = r#"
return {
  api_version = 1,
  description = "demo",
  tools = {
    {
      name = "add",
      description = "adds",
      parameters = {
        type = "object",
        properties = { a = { type = "integer" } },
        required = { "a" },
      },
      run = function(args) return { sum = args.a } end,
    },
  },
  run = function(koru, args)
    local r = koru.ai.run({ prompt = "go", tools = { "add" } })
    return { text = r.text }
  end,
}
"#;

const ENTRY_WITH_MINIMUM: &str = r#"
return {
  api_version = 1,
  description = "demo",
  tools = {
    {
      name = "add",
      description = "adds",
      parameters = {
        type = "object",
        properties = { a = { type = "integer", minimum = 0 } },
        required = { "a" },
      },
      run = function(args) return { sum = args.a } end,
    },
  },
  run = function(koru, args)
    local r = koru.ai.run({ prompt = "go", tools = { "add" } })
    return { text = r.text }
  end,
}
"#;

#[test]
fn tools_unsupported_fails_before_any_request() {
    let fixture = Fixture::new(ENTRY);
    let mut capabilities = ServiceCapabilities::tool_capable("fake");
    capabilities.tools = false;
    let error = fixture
        .run(Box::new(
            FakeAiService::answer("x").capabilities(capabilities),
        ))
        .unwrap_err();
    assert_eq!(error.code(), ErrorCode::UnsupportedCapability);
    assert_eq!(fixture.context.usage().unwrap().model_requests, 0);
}

#[test]
fn unsupported_schema_keyword_fails_before_any_request() {
    let fixture = Fixture::new(ENTRY_WITH_MINIMUM);
    let mut capabilities = ServiceCapabilities::tool_capable("fake");
    capabilities.schema_keywords.remove("minimum");
    let error = fixture
        .run(Box::new(
            FakeAiService::answer("x").capabilities(capabilities),
        ))
        .unwrap_err();
    assert_eq!(error.code(), ErrorCode::UnsupportedCapability);
    assert_eq!(fixture.context.usage().unwrap().model_requests, 0);
}

#[test]
fn supported_request_dispatches_normally() {
    let fixture = Fixture::new(ENTRY);
    let service = FakeAiService::with_rounds(vec![FakeRound::calls(vec![(
        "add".to_owned(),
        JsonValue::Object(
            [("a".to_owned(), JsonValue::Integer(1))]
                .into_iter()
                .collect(),
        ),
    )])]);
    let result = fixture.run(Box::new(service)).unwrap();
    let JsonValue::Object(entries) = result else {
        panic!("expected object");
    };
    assert_eq!(
        entries.get("text"),
        Some(&JsonValue::String("done".to_owned()))
    );
    assert_eq!(fixture.context.usage().unwrap().model_requests, 1);
}
