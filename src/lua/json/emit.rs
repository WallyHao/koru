//! Reverse emission from the Koru JSON model into Lua values.
use super::super::error;
use super::{array_marker, null_marker, object_marker, tag};
use crate::{error::Result, json::JsonValue};
use mlua::{Lua, Value};

/// Convert one JSON value back into an equivalent Lua value.
pub(crate) fn from_json(lua: &Lua, value: &JsonValue) -> Result<Value> {
    match value {
        JsonValue::Null => Ok(Value::LightUserData(null_marker())),
        JsonValue::Bool(flag) => Ok(Value::Boolean(*flag)),
        JsonValue::Integer(number) => Ok(Value::Integer(*number)),
        JsonValue::Number(number) => Ok(Value::Number(*number)),
        JsonValue::String(text) => lua
            .create_string(text)
            .map(Value::String)
            .map_err(|error| error::invalid(format!("cannot build Lua string: {error}"))),
        JsonValue::Array(items) => {
            let table = lua
                .create_table()
                .map_err(|error| error::invalid(format!("cannot build Lua array: {error}")))?;
            for (index, item) in items.iter().enumerate() {
                table
                    .raw_seti(index + 1, from_json(lua, item)?)
                    .map_err(|error| error::invalid(format!("cannot build Lua array: {error}")))?;
            }
            tag(lua, &table, array_marker())
                .map(Value::Table)
                .map_err(|error| error::invalid(format!("cannot tag Lua array: {error}")))
        }
        JsonValue::Object(entries) => {
            let table = lua
                .create_table()
                .map_err(|error| error::invalid(format!("cannot build Lua object: {error}")))?;
            for (key, item) in entries {
                table
                    .raw_set(key.as_str(), from_json(lua, item)?)
                    .map_err(|error| error::invalid(format!("cannot build Lua object: {error}")))?;
            }
            tag(lua, &table, object_marker())
                .map(Value::Table)
                .map_err(|error| error::invalid(format!("cannot tag Lua object: {error}")))
        }
    }
}
