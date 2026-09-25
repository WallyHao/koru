//! Canonical, compact JSON emission from [`JsonValue`].
//!
//! Object keys are already sorted by the model. Numbers are written so that
//! parsing restores the same variant: integral floats keep an explicit `.0`.
use super::JsonValue;

/// Emit compact JSON that [`super::parse`] restores exactly.
pub fn emit(value: &JsonValue) -> String {
    let mut out = String::new();
    write(value, &mut out);
    out
}

fn write(value: &JsonValue, out: &mut String) {
    match value {
        JsonValue::Null => out.push_str("null"),
        JsonValue::Bool(true) => out.push_str("true"),
        JsonValue::Bool(false) => out.push_str("false"),
        JsonValue::Integer(number) => out.push_str(&number.to_string()),
        JsonValue::Number(number) => write_number(*number, out),
        JsonValue::String(text) => write_string(text, out),
        JsonValue::Array(items) => {
            out.push('[');
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                write(item, out);
            }
            out.push(']');
        }
        JsonValue::Object(entries) => {
            out.push('{');
            for (index, (key, item)) in entries.iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                write_string(key, out);
                out.push(':');
                write(item, out);
            }
            out.push('}');
        }
    }
}

fn write_number(number: f64, out: &mut String) {
    if number.is_finite() {
        let mut text = number.to_string();
        if !text.contains(['.', 'e', 'E']) {
            text.push_str(".0");
        }
        out.push_str(&text);
    } else {
        out.push_str("null");
    }
}

fn write_string(text: &str, out: &mut String) {
    out.push('"');
    for ch in text.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            ch if ch < '\u{20}' => {
                out.push_str(&format!("\\u{:04x}", ch as u32));
            }
            ch => out.push(ch),
        }
    }
    out.push('"');
}
