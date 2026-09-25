//! Convert between Lua values and the Koru-owned JSON model.
//!
//! Null and empty containers need an explicit representation: `koru.json.null`
//! is a unique marker, and `koru.json.array`/`koru.json.object` tag a table so an
//! empty array is never confused with an empty object. Untagged tables are
//! inferred structurally; ambiguous or mixed-key tables are rejected. The
//! conversion (`convert`) and the reverse emission (`emit`) live in submodules so
//! each keeps one responsibility.
mod convert;
mod emit;
#[cfg(test)]
mod tests;

use super::error;
use crate::error::Result;
use mlua::{LightUserData, Lua, Table, Value};
use std::{ffi::c_void, ptr};

pub(super) use convert::to_json;
pub(super) use emit::from_json;

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
