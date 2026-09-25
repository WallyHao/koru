//! Validate JSON values against a compiled Koru schema.
use super::{JsonSchema, JsonType, child, schema_error};
use crate::{error::Result, json::JsonValue};
use std::collections::BTreeSet;

impl JsonSchema {
    pub(super) fn validate_at(&self, value: &JsonValue, path: &str) -> Result<()> {
        if !type_allowed(self.types.as_ref(), value) {
            let expected = expected_names(self.types.as_ref());
            return Err(schema_error(
                path,
                format!("expected {expected}, found {}", value.kind()),
            ));
        }
        if let Some(values) = &self.enum_values
            && !values.iter().any(|allowed| allowed == value)
        {
            return Err(schema_error(
                path,
                "value is not one of the allowed enum values",
            ));
        }
        match value {
            JsonValue::Integer(number) => {
                check_range(*number as f64, self.minimum, self.maximum, path)?;
            }
            JsonValue::Number(number) => {
                check_range(*number, self.minimum, self.maximum, path)?;
            }
            JsonValue::String(text) => {
                let length = text.len() as u64;
                check_length(
                    length,
                    self.min_length,
                    self.max_length,
                    path,
                    "string length",
                )?;
            }
            JsonValue::Array(items) => {
                let length = items.len() as u64;
                check_length(length, self.min_items, self.max_items, path, "array length")?;
                if let Some(schema) = &self.items {
                    for (index, item) in items.iter().enumerate() {
                        schema.validate_at(item, &child(path, &index.to_string()))?;
                    }
                }
            }
            JsonValue::Object(entries) => {
                for name in &self.required {
                    if !entries.contains_key(name) {
                        return Err(schema_error(
                            path,
                            format!("missing required property {name:?}"),
                        ));
                    }
                }
                for (name, item) in entries {
                    if let Some(schema) = self.properties.get(name) {
                        schema.validate_at(item, &child(path, name))?;
                    } else if !self.additional_properties {
                        return Err(schema_error(
                            &child(path, name),
                            "additional properties are not allowed",
                        ));
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }
}

pub(super) fn type_allowed(types: Option<&BTreeSet<JsonType>>, value: &JsonValue) -> bool {
    let Some(types) = types else {
        return true;
    };
    match value {
        JsonValue::Null => types.contains(&JsonType::Null),
        JsonValue::Bool(_) => types.contains(&JsonType::Boolean),
        JsonValue::Integer(_) => {
            types.contains(&JsonType::Integer) || types.contains(&JsonType::Number)
        }
        JsonValue::Number(_) => types.contains(&JsonType::Number),
        JsonValue::String(_) => types.contains(&JsonType::String),
        JsonValue::Array(_) => types.contains(&JsonType::Array),
        JsonValue::Object(_) => types.contains(&JsonType::Object),
    }
}

fn expected_names(types: Option<&BTreeSet<JsonType>>) -> String {
    match types {
        None => "any value".to_owned(),
        Some(types) => types
            .iter()
            .map(|kind| kind.name())
            .collect::<Vec<_>>()
            .join(" or "),
    }
}

fn check_range(value: f64, minimum: Option<f64>, maximum: Option<f64>, path: &str) -> Result<()> {
    if let Some(min) = minimum
        && value < min
    {
        return Err(schema_error(
            path,
            format!("value {value} is below the minimum {min}"),
        ));
    }
    if let Some(max) = maximum
        && value > max
    {
        return Err(schema_error(
            path,
            format!("value {value} is above the maximum {max}"),
        ));
    }
    Ok(())
}

fn check_length(
    value: u64,
    minimum: Option<u64>,
    maximum: Option<u64>,
    path: &str,
    what: &str,
) -> Result<()> {
    if let Some(min) = minimum
        && value < min
    {
        return Err(schema_error(
            path,
            format!("{what} {value} is below the minimum {min}"),
        ));
    }
    if let Some(max) = maximum
        && value > max
    {
        return Err(schema_error(
            path,
            format!("{what} {value} is above the maximum {max}"),
        ));
    }
    Ok(())
}
