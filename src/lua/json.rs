//! Convert between Lua values and the Koru-owned JSON model.
//!
//! Null and empty containers need an explicit representation: `koru.json.null`
//! is a unique marker, and `koru.json.array`/`koru.json.object` tag a table so an
//! empty array is never confused with an empty object. Untagged tables are
//! inferred structurally; ambiguous or mixed-key tables are rejected.
use super::error;
use crate::{
    error::Result,
    json::{JsonLimits, JsonValue},
};
use mlua::{LightUserData, Lua, LuaString, Table, Value};
use std::{collections::BTreeSet, ffi::c_void, ptr};

static NULL_BYTE: u8 = 0;
static ARRAY_BYTE: u8 = 0;
static OBJECT_BYTE: u8 = 0;
const TAG_KEY: &str = "__koru_json_tag";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tag {
    Array,
    Object,
}

fn null_marker() -> LightUserData {
    LightUserData(ptr::addr_of!(NULL_BYTE) as *mut c_void)
}
fn array_marker() -> LightUserData {
    LightUserData(ptr::addr_of!(ARRAY_BYTE) as *mut c_void)
}
fn object_marker() -> LightUserData {
    LightUserData(ptr::addr_of!(OBJECT_BYTE) as *mut c_void)
}

/// Build the `koru.json` host table; the caller places it inside `koru`.
pub(super) fn install(lua: &Lua) -> Result<Table> {
    let json = lua
        .create_table()
        .map_err(|error| error::invalid(format!("cannot create koru.json: {error}")))?;
    json.set("null", null_marker())
        .map_err(|error| error::invalid(format!("cannot create koru.json: {error}")))?;
    let array = lua
        .create_function(|lua, table: Table| tag(lua, &table, array_marker()))
        .map_err(|error| error::invalid(format!("cannot create koru.json: {error}")))?;
    let object = lua
        .create_function(|lua, table: Table| tag(lua, &table, object_marker()))
        .map_err(|error| error::invalid(format!("cannot create koru.json: {error}")))?;
    json.set("array", array)
        .map_err(|error| error::invalid(format!("cannot create koru.json: {error}")))?;
    json.set("object", object)
        .map_err(|error| error::invalid(format!("cannot create koru.json: {error}")))?;
    Ok(json)
}

fn tag(lua: &Lua, table: &Table, marker: LightUserData) -> mlua::Result<Table> {
    let metatable = match table.metatable() {
        Some(metatable) => metatable,
        None => {
            let metatable = lua.create_table()?;
            table.set_metatable(Some(metatable.clone()))?;
            metatable
        }
    };
    metatable.raw_set(TAG_KEY, marker)?;
    Ok(table.clone())
}

fn tag_of(table: &Table) -> Option<Tag> {
    let metatable = table.metatable()?;
    let value = metatable.raw_get::<Value>(TAG_KEY).ok()?;
    match value {
        Value::LightUserData(pointer) if pointer.0 == array_marker().0 => Some(Tag::Array),
        Value::LightUserData(pointer) if pointer.0 == object_marker().0 => Some(Tag::Object),
        _ => None,
    }
}

/// Convert one Lua value into JSON under the supplied limits.
pub(super) fn to_json(value: &Value, limits: &JsonLimits) -> Result<JsonValue> {
    let mut state = Convert {
        limits: *limits,
        visited: Vec::new(),
        elements: 0,
        bytes: 0,
    };
    state.value(value, 0)
}

/// Convert one JSON value back into an equivalent Lua value.
pub(super) fn from_json(lua: &Lua, value: &JsonValue) -> Result<Value> {
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

struct Convert {
    limits: JsonLimits,
    visited: Vec<*const c_void>,
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

#[cfg(test)]
mod tests {
    use super::*;

    fn lua_with_json() -> Lua {
        let lua = Lua::new();
        let json = install(&lua).unwrap();
        lua.globals().set("json", json).unwrap();
        lua
    }
    fn eval(lua: &Lua, code: &str, limits: &JsonLimits) -> Result<JsonValue> {
        let value: Value = lua.load(code).eval().unwrap();
        to_json(&value, limits)
    }
    fn default_eval(lua: &Lua, code: &str) -> Result<JsonValue> {
        eval(lua, code, &JsonLimits::default())
    }

    #[test]
    fn round_trips_scalars_and_null() {
        let lua = lua_with_json();
        assert_eq!(default_eval(&lua, "json.null").unwrap(), JsonValue::Null);
        assert_eq!(default_eval(&lua, "true").unwrap(), JsonValue::Bool(true));
        assert_eq!(default_eval(&lua, "42").unwrap(), JsonValue::Integer(42));
        assert_eq!(default_eval(&lua, "1.5").unwrap(), JsonValue::Number(1.5));
        assert_eq!(
            default_eval(&lua, "'hi'").unwrap(),
            JsonValue::String("hi".to_owned())
        );
    }

    #[test]
    fn distinguishes_empty_arrays_and_objects() {
        let lua = lua_with_json();
        assert_eq!(
            default_eval(&lua, "{}").unwrap(),
            JsonValue::Object(Default::default())
        );
        assert_eq!(
            default_eval(&lua, "json.array({})").unwrap(),
            JsonValue::Array(Vec::new())
        );
        assert_eq!(
            default_eval(&lua, "json.object({})").unwrap(),
            JsonValue::Object(Default::default())
        );
    }

    #[test]
    fn round_trips_nested_containers_through_lua() {
        let lua = lua_with_json();
        let source = "{ a = 1, b = { true, json.null }, c = json.array({}) }";
        let json = default_eval(&lua, source).unwrap();
        let back = from_json(&lua, &json).unwrap();
        assert_eq!(to_json(&back, &JsonLimits::default()).unwrap(), json);
    }

    #[test]
    fn rejects_ambiguous_and_mixed_tables() {
        let lua = lua_with_json();
        for code in ["{ 1, 2, x = 3 }", "{ [1] = 1, [3] = 3 }", "{ [0] = 1 }"] {
            assert_eq!(
                default_eval(&lua, code).unwrap_err().code(),
                crate::error::ErrorCode::Validation
            );
        }
    }

    #[test]
    fn rejects_cycles_and_unsupported_values() {
        let lua = lua_with_json();
        assert_eq!(
            default_eval(&lua, "local t = {} t.self = t return t")
                .unwrap_err()
                .code(),
            crate::error::ErrorCode::Validation
        );
        assert_eq!(
            default_eval(&lua, "print").unwrap_err().code(),
            crate::error::ErrorCode::Validation
        );
        assert_eq!(
            default_eval(&lua, "string.char(255)").unwrap_err().code(),
            crate::error::ErrorCode::Validation
        );
        assert_eq!(
            default_eval(&lua, "nil").unwrap_err().code(),
            crate::error::ErrorCode::Validation
        );
    }

    #[test]
    fn rejects_nonfinite_and_unsafe_numbers() {
        let lua = lua_with_json();
        assert_eq!(
            default_eval(&lua, "math.huge").unwrap_err().code(),
            crate::error::ErrorCode::Validation
        );
        assert_eq!(
            default_eval(&lua, "9007199254740993").unwrap_err().code(),
            crate::error::ErrorCode::Validation
        );
    }

    #[test]
    fn enforces_depth_element_and_byte_limits() {
        let lua = lua_with_json();
        let shallow = JsonLimits {
            max_depth: 1,
            ..JsonLimits::default()
        };
        assert_eq!(
            eval(&lua, "{{ 1 }}", &shallow).unwrap_err().code(),
            crate::error::ErrorCode::BudgetExhausted
        );
        let tiny = JsonLimits {
            max_string_bytes: 1,
            ..JsonLimits::default()
        };
        assert_eq!(
            eval(&lua, "'too long'", &tiny).unwrap_err().code(),
            crate::error::ErrorCode::BudgetExhausted
        );
        let few = JsonLimits {
            max_elements: 0,
            ..JsonLimits::default()
        };
        assert_eq!(
            eval(&lua, "{1,2,3}", &few).unwrap_err().code(),
            crate::error::ErrorCode::BudgetExhausted
        );
    }
}
