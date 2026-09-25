//! Encode an AI result into the Lua value the workflow resumes with.
use crate::ai::{AiResult, FinishReason};
use mlua::{Lua, Value};

/// Build the result table handed back to the suspended `koru.ai` call.
pub(super) fn encode_result(lua: &Lua, result: &AiResult) -> mlua::Result<Value> {
    let table = lua.create_table()?;
    table.set("koru_kind", "ai_result")?;
    table.set("text", result.text.as_str())?;
    table.set("finish_reason", finish_reason_name(&result.finish_reason))?;
    table.set("model", result.model.as_str())?;
    table.set("input_tokens", result.usage.input_tokens)?;
    table.set("output_tokens", result.usage.output_tokens)?;
    table.set("request_id", result.request_id.as_deref())?;
    Ok(Value::Table(table))
}

fn finish_reason_name(reason: &FinishReason) -> &str {
    match reason {
        FinishReason::Stop => "stop",
        FinishReason::Length => "length",
        FinishReason::ToolCalls => "tool_calls",
        FinishReason::ContentFilter => "content_filter",
        FinishReason::Other(_) => "other",
    }
}
