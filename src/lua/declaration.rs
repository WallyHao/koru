//! Convert an evaluated Lua table into the Rust-owned declaration contract.
use super::error;
use crate::{
    declaration::{
        Argument, ArgumentType, ArgumentValue, Capabilities, CommandDeclaration, MAX_ARGUMENTS,
    },
    error::Result,
};
use mlua::{Function, Table, Value};

/// Extract and validate the declaration table plus its retained `run` function.
pub(super) fn convert(table: &Table) -> Result<(CommandDeclaration, Function)> {
    let mut api_version = None;
    let mut description = None;
    let mut arguments = Vec::new();
    let mut capabilities = Capabilities::default();
    let mut run = None;
    for pair in table.pairs::<String, Value>() {
        let (key, value) =
            pair.map_err(|error| error::invalid(format!("invalid declaration: {error}")))?;
        match key.as_str() {
            "api_version" => api_version = Some(read_u32(&value, "api_version")?),
            "description" => description = Some(read_string(&value, "description")?),
            "arguments" => arguments = read_arguments(&value)?,
            "capabilities" => capabilities = read_capabilities(&value)?,
            "run" => {
                run =
                    Some(value.as_function().cloned().ok_or_else(|| {
                        error::invalid("declaration field `run` must be a function")
                    })?);
            }
            "tools" => {
                return Err(error::unsupported(
                    "tool declarations are not supported yet",
                ));
            }
            "exemptions" => {
                return Err(error::unsupported(
                    "requested exemptions are not supported yet",
                ));
            }
            other => {
                return Err(error::invalid(format!(
                    "unknown declaration field {other:?}"
                )));
            }
        }
    }
    let api_version =
        api_version.ok_or_else(|| error::invalid("declaration is missing `api_version`"))?;
    let description =
        description.ok_or_else(|| error::invalid("declaration is missing `description`"))?;
    let declaration = CommandDeclaration::new(api_version, description, arguments, capabilities)?;
    let run = run.ok_or_else(|| error::invalid("declaration is missing `run`"))?;
    Ok((declaration, run))
}

fn read_arguments(value: &Value) -> Result<Vec<Argument>> {
    let table = value
        .as_table()
        .ok_or_else(|| error::invalid("declaration field `arguments` must be an array"))?;
    if table.raw_len() > MAX_ARGUMENTS {
        return Err(error::invalid(format!(
            "a command may declare at most {MAX_ARGUMENTS} arguments"
        )));
    }
    let mut entries = Vec::new();
    for pair in table.pairs::<Value, Value>() {
        let (key, value) =
            pair.map_err(|error| error::invalid(format!("invalid arguments: {error}")))?;
        let index = match key {
            Value::Integer(index) if index >= 1 => index,
            _ => return Err(error::invalid("`arguments` must be an array of tables")),
        };
        let entry = value
            .as_table()
            .cloned()
            .ok_or_else(|| error::invalid("each `arguments` entry must be a table"))?;
        entries.push((index, entry));
    }
    entries.sort_by_key(|(index, _)| *index);
    for (position, (index, _)) in entries.iter().enumerate() {
        if *index != position as i64 + 1 {
            return Err(error::invalid(
                "`arguments` entries must be contiguous starting at 1",
            ));
        }
    }
    entries
        .into_iter()
        .map(|(_, entry)| read_argument(&entry))
        .collect()
}

fn read_argument(table: &Table) -> Result<Argument> {
    let mut name = None;
    let mut type_name = None;
    let mut values = None;
    let mut required = false;
    let mut default = None;
    let mut help = None;
    let mut min = None;
    let mut max = None;
    let mut max_len = None;
    for pair in table.pairs::<String, Value>() {
        let (key, value) =
            pair.map_err(|error| error::invalid(format!("invalid argument: {error}")))?;
        match key.as_str() {
            "name" => name = Some(read_string(&value, "argument name")?),
            "type" => type_name = Some(read_string(&value, "argument type")?),
            "values" => values = Some(read_string_array(&value, "values")?),
            "required" => required = read_bool(&value, "required")?,
            "default" => default = Some(read_literal(&value)?),
            "help" => help = Some(read_string(&value, "help")?),
            "min" => min = Some(read_number(&value, "min")?),
            "max" => max = Some(read_number(&value, "max")?),
            "max_len" => max_len = Some(read_u64(&value, "max_len")?),
            other => return Err(error::invalid(format!("unknown argument field {other:?}"))),
        }
    }
    let name = name.ok_or_else(|| error::invalid("an argument is missing `name`"))?;
    let type_name = type_name.ok_or_else(|| error::invalid("an argument is missing `type`"))?;
    let has_values = values.is_some();
    let kind = match type_name.as_str() {
        "string" => ArgumentType::String,
        "integer" => ArgumentType::Integer,
        "number" => ArgumentType::Number,
        "boolean" => ArgumentType::Boolean,
        "enum" => ArgumentType::Enum(
            values.ok_or_else(|| error::invalid("an enum argument needs `values`"))?,
        ),
        other => {
            return Err(error::invalid(format!(
                "unsupported argument type {other:?}"
            )));
        }
    };
    if has_values && !matches!(kind, ArgumentType::Enum(_)) {
        return Err(error::invalid("`values` is only valid for enum arguments"));
    }
    Ok(Argument {
        name,
        kind,
        required,
        default,
        help,
        min,
        max,
        max_len,
    })
}

fn read_capabilities(value: &Value) -> Result<Capabilities> {
    let table = value
        .as_table()
        .ok_or_else(|| error::invalid("`capabilities` must be a table"))?;
    let mut capabilities = Capabilities::default();
    for pair in table.pairs::<String, Value>() {
        let (key, value) =
            pair.map_err(|error| error::invalid(format!("invalid capabilities: {error}")))?;
        match key.as_str() {
            "direct_processes" => {
                capabilities.direct_processes = read_bool(&value, "capabilities.direct_processes")?;
            }
            other => return Err(error::invalid(format!("unknown capability {other:?}"))),
        }
    }
    Ok(capabilities)
}

fn read_literal(value: &Value) -> Result<ArgumentValue> {
    match value {
        Value::String(_) => Ok(ArgumentValue::String(read_string(value, "default")?)),
        Value::Integer(number) => Ok(ArgumentValue::Integer(*number)),
        Value::Number(number) => Ok(ArgumentValue::Number(*number)),
        Value::Boolean(flag) => Ok(ArgumentValue::Boolean(*flag)),
        _ => Err(error::invalid(
            "a default must be a string, integer, number, or boolean",
        )),
    }
}

fn read_string_array(value: &Value, field: &str) -> Result<Vec<String>> {
    let table = value
        .as_table()
        .ok_or_else(|| error::invalid(format!("`{field}` must be an array")))?;
    let mut values = Vec::new();
    for pair in table.pairs::<Value, Value>() {
        let (key, value) =
            pair.map_err(|error| error::invalid(format!("invalid `{field}`: {error}")))?;
        if !matches!(key, Value::Integer(index) if index >= 1) {
            return Err(error::invalid(format!(
                "`{field}` must be an array of strings"
            )));
        }
        values.push(read_string(&value, field)?);
    }
    Ok(values)
}

fn read_string(value: &Value, field: &str) -> Result<String> {
    let text = value
        .as_string()
        .ok_or_else(|| error::invalid(format!("`{field}` must be a string")))?;
    text.to_str()
        .map(|borrowed| borrowed.as_ref().to_owned())
        .map_err(|_| error::invalid(format!("`{field}` must be valid UTF-8")))
}

fn read_bool(value: &Value, field: &str) -> Result<bool> {
    value
        .as_boolean()
        .ok_or_else(|| error::invalid(format!("`{field}` must be a boolean")))
}

fn read_u32(value: &Value, field: &str) -> Result<u32> {
    match value {
        Value::Integer(number) => {
            u32::try_from(*number).map_err(|_| error::invalid(format!("`{field}` is out of range")))
        }
        _ => Err(error::invalid(format!("`{field}` must be an integer"))),
    }
}

fn read_u64(value: &Value, field: &str) -> Result<u64> {
    match value {
        Value::Integer(number) if *number >= 0 => {
            u64::try_from(*number).map_err(|_| error::invalid(format!("`{field}` is out of range")))
        }
        _ => Err(error::invalid(format!(
            "`{field}` must be a nonnegative integer"
        ))),
    }
}

fn read_number(value: &Value, field: &str) -> Result<f64> {
    match value {
        Value::Integer(number) => Ok(*number as f64),
        Value::Number(number) => Ok(*number),
        _ => Err(error::invalid(format!("`{field}` must be a number"))),
    }
}
