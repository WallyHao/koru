//! Structured proposal boundary used by the shell workflow.
use koru::{
    ai::{AiService, fake::FakeAiService},
    error::ErrorCode,
    json::JsonValue,
    lua::LoadedCommand,
    runtime::{ExecutionContext, Limits},
    source::{SourceBundle, SourceLimits},
};
use std::{fs, time::Instant};

const COMMAND: &str = r#"
return {
  api_version = 1,
  description = "structured",
  run = function(koru)
    return koru.ai.ask_json({
      prompt = "propose",
      mode = "prompt_validate",
      schema = "shell_proposal",
    })
  end,
}
"#;

fn run(answer: &str) -> koru::error::Result<JsonValue> {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("proposal.lua"), COMMAND).unwrap();
    let bundle = SourceBundle::capture(root.path(), "proposal", SourceLimits::default()).unwrap();
    let context = ExecutionContext::new(
        "proposal",
        bundle.digest(),
        Limits::default(),
        Instant::now(),
    )
    .unwrap();
    let command = LoadedCommand::load(&bundle, &context).unwrap();
    command.run(
        Box::new(FakeAiService::answer(answer)) as Box<dyn AiService>,
        &JsonValue::Object(Default::default()),
    )
}

#[test]
fn ask_json_returns_only_parsed_schema_validated_data() {
    let value = run(r#"{"script":"printf ok","cwd":"/tmp","explanation":"show files"}"#).unwrap();
    let JsonValue::Object(fields) = value else {
        panic!("expected object")
    };
    assert_eq!(
        fields.get("script"),
        Some(&JsonValue::String("printf ok".into()))
    );
    assert_eq!(
        fields.get("explanation"),
        Some(&JsonValue::String("show files".into()))
    );
}

#[test]
fn ask_json_rejects_malformed_or_schema_invalid_answers() {
    for answer in [
        "```json\n{}\n```",
        r#"{"script":"ok","cwd":"/tmp"}"#,
        r#"{"script":"ok","cwd":"/tmp","explanation":"x","extra":true}"#,
        r#"{"script":"","cwd":"/tmp","explanation":"x"}"#,
        r#"{"script":"x","cwd":"/tmp","explanation":""}"#,
        r#"{"script":"x","cwd":"/tmp","explanation":"x"} trailing"#,
        "",
    ] {
        assert_eq!(
            run(answer).unwrap_err().code(),
            ErrorCode::Validation,
            "{answer:?}"
        );
    }
}

#[test]
fn shell_proposal_schema_rejects_oversized_scripts() {
    let answer = format!(
        "{{\"script\":\"{}\",\"cwd\":\"/tmp\",\"explanation\":\"x\"}}",
        "x".repeat(65 * 1024)
    );
    assert_eq!(run(&answer).unwrap_err().code(), ErrorCode::Validation);
}
