//! Parse `koru.ai.ask`/`koru.ai.run` options into a bounded agent request.
use crate::ai::{AiRequest, ToolSpec};
use mlua::{Table, Value};
use std::collections::BTreeMap;

const DEFAULT_MAX_TURNS: u32 = 8;
const MAX_MAX_TURNS: u32 = 32;
const MAX_PROMPT_BYTES: usize = 256 * 1024;

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
                let text = value
                    .as_string()
                    .ok_or_else(|| mlua::Error::RuntimeError("`prompt` must be a string".into()))?;
                let text = text.to_str().map_err(|_| {
                    mlua::Error::RuntimeError("`prompt` must be valid UTF-8".into())
                })?;
                if text.len() > MAX_PROMPT_BYTES {
                    return Err(mlua::Error::RuntimeError("`prompt` is too long".into()));
                }
                prompt = Some(text.as_ref().to_owned());
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
