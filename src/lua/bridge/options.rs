//! Parse `koru.ai.ask`/`koru.ai.run` options into a bounded agent request.
use super::super::json;
use crate::{
    ai::{AiRequest, ToolSpec},
    json::{self as json_codec, JsonLimits},
    schema::JsonSchema,
};
use mlua::{Table, Value};
use std::{collections::BTreeMap, sync::OnceLock};

const DEFAULT_MAX_TURNS: u32 = 8;
const MAX_MAX_TURNS: u32 = 32;
pub(super) const MAX_PROMPT_BYTES: usize = 256 * 1024;

/// One explicitly prompt-and-validate structured request.
pub(super) struct JsonRequest {
    pub(super) request: AiRequest,
    pub(super) schema: JsonSchema,
}

/// Parse `koru.ai.ask_json` options and build its bounded fallback prompt.
pub(super) fn read_json_options(options: &Table) -> mlua::Result<JsonRequest> {
    let mut prompt = None;
    let mut schema = None;
    let mut mode = None;
    for pair in options.pairs::<String, Value>() {
        let (key, value) = pair?;
        match key.as_str() {
            "prompt" => prompt = Some(read_text(&value, "prompt")?),
            "schema" => schema = Some(read_json_schema(&value)?),
            "mode" => mode = Some(read_text(&value, "mode")?),
            other => {
                return Err(mlua::Error::RuntimeError(format!(
                    "unknown ai.ask_json option {other:?}"
                )));
            }
        }
    }
    let prompt =
        prompt.ok_or_else(|| mlua::Error::RuntimeError("ai.ask_json needs a `prompt`".into()))?;
    let schema =
        schema.ok_or_else(|| mlua::Error::RuntimeError("ai.ask_json needs a `schema`".into()))?;
    if mode.as_deref() != Some("prompt_validate") {
        return Err(mlua::Error::RuntimeError(
            "ai.ask_json supports only mode `prompt_validate`".into(),
        ));
    }
    let schema_text = json_codec::emit(schema.document());
    let request_prompt = format!(
        "{prompt}\n\nReturn exactly one JSON value matching this schema. Do not use Markdown fences or add prose.\nSchema: {schema_text}"
    );
    if request_prompt.len() > MAX_PROMPT_BYTES {
        return Err(mlua::Error::RuntimeError(
            "structured prompt is too long".into(),
        ));
    }
    Ok(JsonRequest {
        request: AiRequest {
            prompt: request_prompt,
            tools: Vec::new(),
            max_turns: 1,
        },
        schema,
    })
}

fn read_json_schema(value: &Value) -> mlua::Result<JsonSchema> {
    if let Some(name) = value.as_string() {
        let name = name
            .to_str()
            .map_err(|_| mlua::Error::RuntimeError("schema name must be valid UTF-8".into()))?;
        return match name.as_ref() {
            "shell_proposal" => Ok(shell_proposal_schema().clone()),
            "commit_plan" => Ok(commit_plan_schema().clone()),
            other => Err(mlua::Error::RuntimeError(format!(
                "unknown built-in JSON schema {other:?}"
            ))),
        };
    }
    let document = json::to_json(value, &JsonLimits::default()).map_err(runtime_error)?;
    JsonSchema::compile(&document, &JsonLimits::default()).map_err(runtime_error)
}

fn commit_plan_schema() -> &'static JsonSchema {
    static SCHEMA: OnceLock<JsonSchema> = OnceLock::new();
    SCHEMA.get_or_init(|| {
        let document = crate::json::parse(
            br#"{"type":"object","properties":{"groups":{"type":"array","minItems":1,"maxItems":64,"items":{"type":"object","properties":{"changes":{"type":"array","minItems":1,"maxItems":64,"items":{"type":"string","minLength":1,"maxLength":24}},"message":{"type":"string","minLength":1,"maxLength":72},"rationale":{"type":"string","maxLength":512}},"required":["changes","message"],"additionalProperties":false}}},"required":["groups"],"additionalProperties":false}"#,
            &JsonLimits::default(),
        )
        .expect("the built-in commit plan schema is valid JSON");
        JsonSchema::compile(&document, &JsonLimits::default())
            .expect("the built-in commit plan schema is supported")
    })
}

fn shell_proposal_schema() -> &'static JsonSchema {
    static SCHEMA: OnceLock<JsonSchema> = OnceLock::new();
    SCHEMA.get_or_init(|| {
        let document = crate::json::parse(
            br#"{"type":"object","properties":{"script":{"type":"string","minLength":1,"maxLength":65536},"cwd":{"type":"string","minLength":1,"maxLength":4096},"explanation":{"type":"string","minLength":1,"maxLength":1024}},"required":["script","cwd","explanation"],"additionalProperties":false}"#,
            &JsonLimits::default(),
        )
        .expect("the built-in shell proposal schema is valid JSON");
        JsonSchema::compile(&document, &JsonLimits::default())
            .expect("the built-in shell proposal schema is supported")
    })
}

/// Parse the `koru.ai.run` option table, rejecting unknown tools and options.
pub(super) fn read_run_options(
    options: &Table,
    specs: &BTreeMap<String, ToolSpec>,
) -> mlua::Result<AiRequest> {
    let mut prompt = None;
    let mut tool_names = Vec::new();
    let mut max_turns = DEFAULT_MAX_TURNS;
    for pair in options.pairs::<String, Value>() {
        let (key, value) = pair?;
        match key.as_str() {
            "prompt" => {
                prompt = Some(read_text(&value, "prompt")?);
            }
            "tools" => {
                let list = value
                    .as_table()
                    .ok_or_else(|| mlua::Error::RuntimeError("`tools` must be an array".into()))?;
                for item in list.sequence_values::<String>() {
                    let name = item?;
                    if !specs.contains_key(&name) {
                        return Err(mlua::Error::RuntimeError(format!("unknown tool {name:?}")));
                    }
                    tool_names.push(name);
                }
            }
            "max_turns" => {
                let turns = value.as_integer().ok_or_else(|| {
                    mlua::Error::RuntimeError("`max_turns` must be an integer".into())
                })?;
                if !(1..=i64::from(MAX_MAX_TURNS)).contains(&turns) {
                    return Err(mlua::Error::RuntimeError(
                        "`max_turns` is out of range".into(),
                    ));
                }
                max_turns = turns as u32;
            }
            other => {
                return Err(mlua::Error::RuntimeError(format!(
                    "unknown ai.run option {other:?}"
                )));
            }
        }
    }
    let prompt =
        prompt.ok_or_else(|| mlua::Error::RuntimeError("ai.run needs a `prompt`".into()))?;
    let mut tools = Vec::with_capacity(tool_names.len());
    for name in tool_names {
        if let Some(spec) = specs.get(&name) {
            tools.push(spec.clone());
        }
    }
    Ok(AiRequest {
        prompt,
        tools,
        max_turns,
    })
}

fn read_text(value: &Value, field: &str) -> mlua::Result<String> {
    let text = value
        .as_string()
        .ok_or_else(|| mlua::Error::RuntimeError(format!("`{field}` must be a string")))?;
    let text = text
        .to_str()
        .map_err(|_| mlua::Error::RuntimeError(format!("`{field}` must be valid UTF-8")))?;
    if text.len() > MAX_PROMPT_BYTES {
        return Err(mlua::Error::RuntimeError(format!("`{field}` is too long")));
    }
    Ok(text.as_ref().to_owned())
}

fn runtime_error(error: crate::error::KoruError) -> mlua::Error {
    mlua::Error::RuntimeError(error.message().to_owned())
}
