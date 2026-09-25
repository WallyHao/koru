//! Compile a bounded JSON schema document into the Koru schema model.
use super::read::{
    read_bool, read_enum, read_number, read_properties, read_required, read_types, read_u64,
};
use super::{JsonSchema, JsonType, child, schema_error};
use crate::{
    error::Result,
    json::{JsonLimits, JsonValue, MAX_SAFE_INTEGER},
};
use std::collections::{BTreeMap, BTreeSet};

impl JsonSchema {
    pub(super) fn compile_at(
        document: &JsonValue,
        limits: &JsonLimits,
        depth: usize,
        path: &str,
    ) -> Result<Self> {
        limits.check_depth(depth)?;
        let JsonValue::Object(entries) = document else {
            return Err(schema_error(path, "a JSON schema must be an object"));
        };
        let mut types = None;
        let mut enum_values = None;
        let mut minimum = None;
        let mut maximum = None;
        let mut min_length = None;
        let mut max_length = None;
        let mut items = None;
        let mut min_items = None;
        let mut max_items = None;
        let mut properties = BTreeMap::new();
        let mut required = BTreeSet::new();
        let mut additional_properties = true;
        let mut object_keywords = false;
        let mut array_keywords = false;
        let mut string_keywords = false;
        let mut number_keywords = false;
        let mut keywords = BTreeSet::new();
        for (keyword, value) in entries {
            if let Some(name) = super::SUPPORTED_KEYWORDS
                .iter()
                .find(|name| **name == keyword.as_str())
            {
                keywords.insert(*name);
            }
            match keyword.as_str() {
                "type" => types = Some(read_types(value, path)?),
                "enum" => enum_values = Some(read_enum(value, path)?),
                "minimum" => {
                    minimum = Some(read_number(value, path, keyword)?);
                    number_keywords = true;
                }
                "maximum" => {
                    maximum = Some(read_number(value, path, keyword)?);
                    number_keywords = true;
                }
                "minLength" => {
                    min_length = Some(read_u64(value, path, keyword)?);
                    string_keywords = true;
                }
                "maxLength" => {
                    max_length = Some(read_u64(value, path, keyword)?);
                    string_keywords = true;
                }
                "items" => {
                    items = Some(Box::new(Self::compile_at(
                        value,
                        limits,
                        depth + 1,
                        &child(path, keyword),
                    )?));
                    array_keywords = true;
                }
                "minItems" => {
                    min_items = Some(read_u64(value, path, keyword)?);
                    array_keywords = true;
                }
                "maxItems" => {
                    max_items = Some(read_u64(value, path, keyword)?);
                    array_keywords = true;
                }
                "properties" => {
                    properties = read_properties(value, limits, depth, path)?;
                    object_keywords = true;
                }
                "required" => {
                    required = read_required(value, path)?;
                    object_keywords = true;
                }
                "additionalProperties" => {
                    additional_properties = read_bool(value, path, keyword)?;
                    object_keywords = true;
                }
                "description" => {
                    if !matches!(value, JsonValue::String(_)) {
                        return Err(schema_error(path, "`description` must be a string"));
                    }
                }
                other => {
                    return Err(schema_error(
                        path,
                        format!("unsupported schema keyword {other:?}"),
                    ));
                }
            }
        }
        if let Some(items) = &items {
            keywords.extend(items.keywords.iter().copied());
        }
        for schema in properties.values() {
            keywords.extend(schema.keywords.iter().copied());
        }
        if enum_values.is_some() && types.is_none() {
            types = Some(enum_types(enum_values.as_deref().unwrap_or_default()));
        }
        let types = resolve_types(
            types,
            object_keywords,
            array_keywords,
            string_keywords,
            number_keywords,
            path,
        )?;
        if let Some(values) = &enum_values {
            if values.is_empty() {
                return Err(schema_error(path, "`enum` must not be empty"));
            }
            for value in values {
                if !super::validate::type_allowed(types.as_ref(), value) {
                    return Err(schema_error(
                        path,
                        "each `enum` value must match the declared `type`",
                    ));
                }
            }
        }
        check_bounds(minimum, maximum, path, "minimum")?;
        check_bounds(
            min_length.map(|value| value as f64),
            max_length.map(|value| value as f64),
            path,
            "minLength",
        )?;
        check_bounds(
            min_items.map(|value| value as f64),
            max_items.map(|value| value as f64),
            path,
            "minItems",
        )?;
        if let Some(types) = &types
            && types.contains(&JsonType::Integer)
            && !types.contains(&JsonType::Number)
        {
            for bound in [minimum, maximum].into_iter().flatten() {
                if bound.fract() != 0.0
                    || !(-(MAX_SAFE_INTEGER as f64)..=(MAX_SAFE_INTEGER as f64)).contains(&bound)
                {
                    return Err(schema_error(
                        path,
                        "integer bounds must be whole numbers in the safe range",
                    ));
                }
            }
        }
        for name in &required {
            if !properties.contains_key(name) {
                return Err(schema_error(
                    path,
                    format!("`required` names undeclared property {name:?}"),
                ));
            }
        }
        Ok(Self {
            types,
            enum_values,
            minimum,
            maximum,
            min_length,
            max_length,
            items,
            min_items,
            max_items,
            properties,
            required,
            additional_properties,
            keywords,
        })
    }
}

fn resolve_types(
    explicit: Option<BTreeSet<JsonType>>,
    object: bool,
    array: bool,
    string: bool,
    number: bool,
    path: &str,
) -> Result<Option<BTreeSet<JsonType>>> {
    match explicit {
        Some(types) => {
            if object && !types.contains(&JsonType::Object) {
                return Err(schema_error(path, "object keywords require `object` type"));
            }
            if array && !types.contains(&JsonType::Array) {
                return Err(schema_error(path, "array keywords require `array` type"));
            }
            if string && !types.contains(&JsonType::String) {
                return Err(schema_error(path, "string keywords require `string` type"));
            }
            if number && !types.contains(&JsonType::Integer) && !types.contains(&JsonType::Number) {
                return Err(schema_error(
                    path,
                    "numeric keywords require an `integer` or `number` type",
                ));
            }
            Ok(Some(types))
        }
        None => {
            let mut implied = BTreeSet::new();
            if object {
                implied.insert(JsonType::Object);
            }
            if array {
                implied.insert(JsonType::Array);
            }
            if string {
                implied.insert(JsonType::String);
            }
            if number {
                implied.insert(JsonType::Integer);
                implied.insert(JsonType::Number);
            }
            if implied.is_empty() {
                Ok(None)
            } else {
                Ok(Some(implied))
            }
        }
    }
}

fn enum_types(values: &[JsonValue]) -> BTreeSet<JsonType> {
    let mut types = BTreeSet::new();
    for value in values {
        match value {
            JsonValue::Null => {
                types.insert(JsonType::Null);
            }
            JsonValue::Bool(_) => {
                types.insert(JsonType::Boolean);
            }
            JsonValue::Integer(_) => {
                types.insert(JsonType::Integer);
            }
            JsonValue::Number(_) => {
                types.insert(JsonType::Number);
            }
            JsonValue::String(_) => {
                types.insert(JsonType::String);
            }
            JsonValue::Array(_) => {
                types.insert(JsonType::Array);
            }
            JsonValue::Object(_) => {
                types.insert(JsonType::Object);
            }
        }
    }
    types
}

fn check_bounds(
    minimum: Option<f64>,
    maximum: Option<f64>,
    path: &str,
    keyword: &str,
) -> Result<()> {
    if let (Some(min), Some(max)) = (minimum, maximum)
        && min > max
    {
        return Err(schema_error(
            path,
            format!("`{keyword}` greater than its maximum"),
        ));
    }
    Ok(())
}
