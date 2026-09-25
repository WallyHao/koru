//! Forward conversion from Lua values into the Koru JSON model.
use super::super::error;
use super::{Tag, null_marker, tag_of};
use crate::{
    error::Result,
    json::{JsonLimits, JsonValue},
};
use mlua::{LuaString, Table, Value};
use std::collections::BTreeSet;

/// Convert one Lua value into JSON under the supplied limits.
pub(crate) fn to_json(value: &Value, limits: &JsonLimits) -> Result<JsonValue> {
    let mut state = Convert {
        limits: *limits,
        visited: Vec::new(),
        elements: 0,
        bytes: 0,
    };
    state.value(value, 0)
}

struct Convert {
    limits: JsonLimits,
    visited: Vec<*const std::ffi::c_void>,
    elements: usize,
    bytes: usize,
}
impl Convert {
    fn value(&mut self, value: &Value, depth: usize) -> Result<JsonValue> {
        self.limits.check_depth(depth)?;
        match value {
            Value::Nil => Err(error::invalid(
                "Lua nil is not JSON null; use koru.json.null",
            )),
            Value::Boolean(flag) => Ok(JsonValue::Bool(*flag)),
            Value::Integer(number) => {
                if !(-crate::json::MAX_SAFE_INTEGER..=crate::json::MAX_SAFE_INTEGER)
                    .contains(number)
                {
                    return Err(error::invalid("JSON integer is outside the safe range"));
                }
                self.charge(8)?;
                Ok(JsonValue::Integer(*number))
            }
            Value::Number(number) => {
                if !number.is_finite() {
                    return Err(error::invalid("JSON numbers must be finite"));
                }
                self.charge(8)?;
                Ok(JsonValue::Number(*number))
            }
            Value::String(text) => self.string(text),
            Value::Table(table) => self.table(table, depth),
            Value::LightUserData(pointer) if pointer.0 == null_marker().0 => Ok(JsonValue::Null),
            _ => Err(error::invalid("unsupported Lua value for JSON conversion")),
        }
    }
    fn string(&mut self, text: &LuaString) -> Result<JsonValue> {
        let borrowed = text
            .to_str()
            .map_err(|_| error::invalid("JSON strings must be valid UTF-8"))?;
        let value = borrowed.as_ref();
        self.limits.check_string_bytes(value.len())?;
        self.charge(value.len())?;
        Ok(JsonValue::String(value.to_owned()))
    }
    fn table(&mut self, table: &Table, depth: usize) -> Result<JsonValue> {
        let pointer = table.to_pointer();
        if self.visited.contains(&pointer) {
            return Err(error::invalid("JSON value contains a cycle"));
        }
        self.visited.push(pointer);
        let result = match tag_of(table) {
            Some(Tag::Array) => self.array(table, depth),
            Some(Tag::Object) => self.object(table, depth),
            None => match classify(table)? {
                Shape::Array => self.array(table, depth),
                Shape::Object => self.object(table, depth),
            },
        };
        self.visited.pop();
        result
    }
    fn array(&mut self, table: &Table, depth: usize) -> Result<JsonValue> {
        let length = table.raw_len();
        let mut items = Vec::with_capacity(length.min(1024));
        for index in 1..=length {
            let value: Value = table
                .raw_get(index)
                .map_err(|error| error::invalid(format!("invalid JSON array: {error}")))?;
            items.push(self.value(&value, depth + 1)?);
        }
        Ok(JsonValue::Array(items))
    }
    fn object(&mut self, table: &Table, depth: usize) -> Result<JsonValue> {
        let mut entries = std::collections::BTreeMap::new();
        for pair in table.pairs::<Value, Value>() {
            let (key, value) =
                pair.map_err(|error| error::invalid(format!("invalid JSON object: {error}")))?;
            let Value::String(key) = key else {
                return Err(error::invalid("JSON object keys must be strings"));
            };
            let key = key
                .to_str()
                .map_err(|_| error::invalid("JSON object keys must be valid UTF-8"))?
                .as_ref()
                .to_owned();
            self.limits.check_key_bytes(key.len())?;
            self.charge(key.len())?;
            if entries
                .insert(key, self.value(&value, depth + 1)?)
                .is_some()
            {
                return Err(error::invalid("JSON object contains a duplicate key"));
            }
        }
        Ok(JsonValue::Object(entries))
    }
    fn charge(&mut self, bytes: usize) -> Result<()> {
        self.elements = self.elements.saturating_add(1);
        self.limits.check_elements(self.elements)?;
        self.bytes = self.bytes.saturating_add(bytes);
        self.limits.check_bytes(self.bytes)
    }
}

enum Shape {
    Array,
    Object,
}
fn classify(table: &Table) -> Result<Shape> {
    let length = table.raw_len();
    let mut integers = BTreeSet::new();
    let mut strings = 0usize;
    for pair in table.pairs::<Value, Value>() {
        let (key, _) =
            pair.map_err(|error| error::invalid(format!("invalid JSON table: {error}")))?;
        match key {
            Value::Integer(index) if index >= 1 => {
                integers.insert(index);
            }
            Value::Integer(_) => {
                return Err(error::invalid("JSON array indices must start at 1"));
            }
            Value::String(_) => strings += 1,
            _ => return Err(error::invalid("JSON object keys must be strings")),
        }
    }
    if strings == 0 && integers.is_empty() {
        return Ok(Shape::Object);
    }
    if strings == 0 {
        let contiguous = integers.len() == length
            && integers.iter().next() == Some(&1)
            && integers.iter().next_back() == Some(&(length as i64));
        if contiguous {
            return Ok(Shape::Array);
        }
        return Err(error::invalid("JSON array is sparse or not contiguous"));
    }
    if integers.is_empty() {
        return Ok(Shape::Object);
    }
    Err(error::invalid("JSON table mixes array and object keys"))
}
