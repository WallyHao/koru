//! Read and bound individual JSON schema keywords during compilation.
use super::{JsonSchema, JsonType, child, schema_error};
use crate::{
    error::Result,
    json::{JsonLimits, JsonValue},
};
use std::collections::{BTreeMap, BTreeSet};

pub(super) fn read_types(value: &JsonValue, path: &str) -> Result<BTreeSet<JsonType>> {
    let names: Vec<&str> = match value {
        JsonValue::String(name) => vec![name.as_str()],
        JsonValue::Array(items) => {
            let mut names = Vec::with_capacity(items.len());
            for item in items {
                match item {
                    JsonValue::String(name) => names.push(name.as_str()),
                    _ => return Err(schema_error(path, "`type` entries must be strings")),
                }
            }
            names
        }
        _ => {
            return Err(schema_error(
                path,
                "`type` must be a string or array of strings",
            ));
        }
    };
    if names.is_empty() {
        return Err(schema_error(path, "`type` must name at least one type"));
    }
    let mut types = BTreeSet::new();
    for name in names {
        let kind = JsonType::from_name(name)
            .ok_or_else(|| schema_error(path, format!("unknown JSON type {name:?}")))?;
        if !types.insert(kind) {
            return Err(schema_error(path, format!("duplicate JSON type {name:?}")));
        }
    }
    Ok(types)
}

pub(super) fn read_enum(value: &JsonValue, path: &str) -> Result<Vec<JsonValue>> {
    let JsonValue::Array(items) = value else {
        return Err(schema_error(path, "`enum` must be an array"));
    };
    let mut seen: Vec<&JsonValue> = Vec::new();
    for item in items {
        if seen.contains(&item) {
            return Err(schema_error(path, "`enum` values must be unique"));
        }
        seen.push(item);
    }
    Ok(items.clone())
}

pub(super) fn read_properties(
    value: &JsonValue,
    limits: &JsonLimits,
    depth: usize,
    path: &str,
) -> Result<BTreeMap<String, JsonSchema>> {
    let JsonValue::Object(entries) = value else {
        return Err(schema_error(path, "`properties` must be an object"));
    };
    let mut properties = BTreeMap::new();
    for (name, document) in entries {
        let schema = JsonSchema::compile_at(document, limits, depth + 1, &child(path, name))?;
        properties.insert(name.clone(), schema);
    }
    Ok(properties)
}

pub(super) fn read_required(value: &JsonValue, path: &str) -> Result<BTreeSet<String>> {
    let JsonValue::Array(items) = value else {
        return Err(schema_error(path, "`required` must be an array"));
    };
    let mut names = BTreeSet::new();
    for item in items {
        let JsonValue::String(name) = item else {
            return Err(schema_error(path, "`required` entries must be strings"));
        };
        if !names.insert(name.clone()) {
            return Err(schema_error(path, "`required` entries must be unique"));
        }
    }
    Ok(names)
}

pub(super) fn read_bool(value: &JsonValue, path: &str, keyword: &str) -> Result<bool> {
    match value {
        JsonValue::Bool(flag) => Ok(*flag),
        _ => Err(schema_error(path, format!("`{keyword}` must be a boolean"))),
    }
}

pub(super) fn read_u64(value: &JsonValue, path: &str, keyword: &str) -> Result<u64> {
    match value {
        JsonValue::Integer(number) if *number >= 0 => Ok(*number as u64),
        _ => Err(schema_error(
            path,
            format!("`{keyword}` must be a nonnegative integer"),
        )),
    }
}

pub(super) fn read_number(value: &JsonValue, path: &str, keyword: &str) -> Result<f64> {
    let number = match value {
        JsonValue::Integer(number) => *number as f64,
        JsonValue::Number(number) => *number,
        _ => return Err(schema_error(path, format!("`{keyword}` must be a number"))),
    };
    if !number.is_finite() {
        return Err(schema_error(path, format!("`{keyword}` must be finite")));
    }
    Ok(number)
}
