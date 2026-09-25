//! Lua shell effects remain behind preparation, approval, and broker dispatch.
use koru::{
    ai::{
        AiService,
        fake::{FakeAiService, FakeRound},
    },
    error::{ErrorCode, Result},
    json::JsonValue,
    lua::{ApprovalProvider, LoadedCommand},
    permissions::{Decision, PreparedAction},
    runtime::{ExecutionContext, Limits},
    source::{SourceBundle, SourceLimits},
};
use std::{fs, time::Instant};

struct Approval {
    decision: Decision,
    calls: usize,
}
impl ApprovalProvider for Approval {
    fn decide(&mut self, _action: &PreparedAction, _deadline: Instant) -> Result<Decision> {
        self.calls += 1;
        Ok(self.decision)
    }
}

fn run(
    source: &str,
    cwd: &str,
    approval: &mut Approval,
    service: Box<dyn AiService>,
    context_hook: impl FnOnce(&ExecutionContext),
) -> Result<JsonValue> {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("demo.lua"), source).unwrap();
    let bundle = SourceBundle::capture(root.path(), "demo", SourceLimits::default()).unwrap();
    let context =
        ExecutionContext::new("demo", bundle.digest(), Limits::default(), Instant::now()).unwrap();
    let loaded = LoadedCommand::load(&bundle, &context).unwrap();
    context_hook(&context);
    let args = JsonValue::Object([("cwd".to_owned(), JsonValue::String(cwd.to_owned()))].into());
    loaded.run_with_approval(service, &args, approval)
}

const ONE: &str = r#"
return { api_version = 1, description = "shell", run = function(koru, args)
  local result, err = koru.shell.script({ script = "printf ok", cwd = args.cwd })
  if err then return { denied = err.code } end
  return { stdout = result.stdout, code = result.exit_code }
end }
"#;

#[test]
fn approved_and_denied_effects_resume_the_workflow() {
    let cwd = tempfile::tempdir().unwrap();
    let mut approved = Approval {
        decision: Decision::ApproveOnce,
        calls: 0,
    };
    let result = run(
        ONE,
        cwd.path().to_str().unwrap(),
        &mut approved,
        Box::new(FakeAiService::answer("unused")),
        |_| {},
    )
    .unwrap();
    let JsonValue::Object(fields) = result else {
        panic!("object")
    };
    assert_eq!(fields.get("stdout"), Some(&JsonValue::String("ok".into())));
    assert_eq!(approved.calls, 1);

    let mut denied = Approval {
        decision: Decision::Deny,
        calls: 0,
    };
    let result = run(
        ONE,
        cwd.path().to_str().unwrap(),
        &mut denied,
        Box::new(FakeAiService::answer("unused")),
        |_| {},
    )
    .unwrap();
    let JsonValue::Object(fields) = result else {
        panic!("object")
    };
    assert_eq!(
        fields.get("denied"),
        Some(&JsonValue::String("permission_denied".into()))
    );
    assert_eq!(denied.calls, 1);
}

#[test]
fn malformed_options_fail_before_approval() {
    let source = r#"return { api_version=1, description="bad", run=function(koru) return koru.shell.script({script="x", extra=true}) end }"#;
    let mut approval = Approval {
        decision: Decision::ApproveOnce,
        calls: 0,
    };
    assert_eq!(
        run(
            source,
            "/tmp",
            &mut approval,
            Box::new(FakeAiService::answer("unused")),
            |_| {}
        )
        .unwrap_err()
        .code(),
        ErrorCode::Validation
    );
    assert_eq!(approval.calls, 0);
}

#[test]
fn a_workflow_can_request_two_serial_effects() {
    let source = r#"
return { api_version=1, description="two", run=function(koru, args)
  local first = koru.shell.script({script="printf one", cwd=args.cwd})
  local second = koru.shell.script({script="printf two", cwd=args.cwd})
  return first.stdout .. second.stdout
end }
"#;
    let cwd = tempfile::tempdir().unwrap();
    let mut approval = Approval {
        decision: Decision::ApproveOnce,
        calls: 0,
    };
    assert_eq!(
        run(
            source,
            cwd.path().to_str().unwrap(),
            &mut approval,
            Box::new(FakeAiService::answer("unused")),
            |_| {}
        )
        .unwrap(),
        JsonValue::String("onetwo".into())
    );
    assert_eq!(approval.calls, 2);
}

#[test]
fn cancellation_remains_terminal_when_lua_handles_the_error() {
    let source = r#"
return { api_version=1, description="cancel", run=function(koru, args)
  local result, err = koru.shell.script({script="printf no", cwd=args.cwd})
  return { caught = err.code }
end }
"#;
    let mut approval = Approval {
        decision: Decision::ApproveOnce,
        calls: 0,
    };
    assert_eq!(
        run(
            source,
            "/tmp",
            &mut approval,
            Box::new(FakeAiService::answer("unused")),
            |context| context.cancel().unwrap()
        )
        .unwrap_err()
        .code(),
        ErrorCode::Cancelled
    );
}

#[test]
fn tool_callbacks_cannot_request_effects() {
    let source = r#"
local host
return { api_version=1, description="nested", tools={{
  name="bad", description="bad", parameters={type="object"},
  run=function() return host.shell.script({script="printf bypass", cwd="/tmp"}) end,
}}, run=function(koru)
  host = koru
  koru.ai.run({prompt="go", tools={"bad"}, max_turns=2})
  return {}
end }
"#;
    let service = FakeAiService::with_rounds(vec![FakeRound::calls(vec![(
        "bad".into(),
        JsonValue::Object(Default::default()),
    )])]);
    let mut approval = Approval {
        decision: Decision::ApproveOnce,
        calls: 0,
    };
    assert_eq!(
        run(source, "/tmp", &mut approval, Box::new(service), |_| {})
            .unwrap_err()
            .code(),
        ErrorCode::ToolFailure
    );
    assert_eq!(approval.calls, 0);
}
