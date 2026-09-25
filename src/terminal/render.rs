//! Human-readable rendering of one workflow result.
//!
//! Command output is the intended product, so the terminal adapter routes
//! captured stdout back to standard output, diagnostics to standard error, and
//! never prints a raw JSON envelope for the reference shell and commit
//! workflows. Unknown result shapes fall back to indented JSON so custom
//! workflows keep a stable machine-readable form.
use super::style::Style;
use crate::{
    error::{KoruError, Result},
    json::{JsonValue, emit},
};
use std::io::{self, Write};

/// Render one workflow result to the process standard streams.
pub fn render_result(value: &JsonValue) -> Result<()> {
    let stdout = io::stdout();
    let stderr = io::stderr();
    let mut out = stdout.lock();
    let mut err = stderr.lock();
    render_to(value, &mut out, &mut err, Style::stdout(), Style::stderr())
        .map_err(|error| KoruError::io("cannot write workflow output", error))
}

/// The process exit status implied by one workflow result.
///
/// A shell result propagates the child's exit code, or `128 + signal` when the
/// child was terminated. A failed effect envelope exits nonzero; every other
/// shape exits zero so ordinary workflows keep the existing status.
pub fn exit_code(value: &JsonValue) -> i32 {
    let JsonValue::Object(fields) = value else {
        return 0;
    };
    if is_error_envelope(fields) {
        return 1;
    }
    if !is_shell_result(fields) {
        return 0;
    }
    if let Some(JsonValue::Integer(signal)) = fields.get("signal") {
        return clamp_status(128 + i32::try_from(*signal).unwrap_or(0));
    }
    match fields.get("exit_code") {
        Some(JsonValue::Integer(code)) => clamp_status(i32::try_from(*code).unwrap_or(1)),
        _ => 0,
    }
}

fn clamp_status(code: i32) -> i32 {
    code.clamp(0, 255)
}

/// Escape control characters in untrusted text while keeping line breaks.
fn escape_text(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        if ch.is_control() && ch != '\n' && ch != '\t' {
            out.extend(ch.escape_default());
        } else {
            out.push(ch);
        }
    }
    out
}

fn render_to(
    value: &JsonValue,
    out: &mut dyn Write,
    err: &mut dyn Write,
    out_style: Style,
    err_style: Style,
) -> io::Result<()> {
    match value {
        JsonValue::Null => Ok(()),
        JsonValue::String(text) => {
            let text = escape_text(text);
            out.write_all(text.as_bytes())?;
            if text.ends_with('\n') {
                Ok(())
            } else {
                out.write_all(b"\n")
            }
        }
        JsonValue::Object(fields) if is_shell_result(fields) => {
            render_shell(fields, out, err, err_style)
        }
        JsonValue::Object(fields) if is_error_envelope(fields) => {
            render_error(fields, err, err_style)
        }
        JsonValue::Object(fields) if is_commit_plan(fields) => {
            render_commit(fields, out, out_style)
        }
        other => write_json(other, 0, out, out_style),
    }
}

fn is_shell_result(fields: &std::collections::BTreeMap<String, JsonValue>) -> bool {
    fields.contains_key("stdout")
        && fields.contains_key("stderr")
        && (fields.contains_key("exit_code") || fields.contains_key("signal"))
}

fn is_error_envelope(fields: &std::collections::BTreeMap<String, JsonValue>) -> bool {
    matches!(fields.get("error"), Some(JsonValue::Object(_))) && !is_shell_result(fields)
}

fn is_commit_plan(fields: &std::collections::BTreeMap<String, JsonValue>) -> bool {
    fields.contains_key("status") && fields.contains_key("groups")
}

fn render_shell(
    fields: &std::collections::BTreeMap<String, JsonValue>,
    out: &mut dyn Write,
    err: &mut dyn Write,
    err_style: Style,
) -> io::Result<()> {
    let mut ended_with_newline = true;
    if let Some(JsonValue::String(text)) = fields.get("stdout") {
        out.write_all(text.as_bytes())?;
        if !text.is_empty() {
            ended_with_newline = text.ends_with('\n');
        }
        out.flush()?;
    }
    if let Some(JsonValue::String(text)) = fields.get("stderr") {
        err.write_all(text.as_bytes())?;
        if !text.is_empty() {
            ended_with_newline = text.ends_with('\n');
        }
    }
    if !ended_with_newline {
        // Keep status and truncation notices on their own line.
        err.write_all(b"\n")?;
    }
    if is_true(fields.get("stdout_truncated")) || is_true(fields.get("stderr_truncated")) {
        err.write_all(err_style.yellow("koru: output truncated\n").as_bytes())?;
    }
    if let Some(JsonValue::Integer(signal)) = fields.get("signal") {
        let line = format!("koru: shell terminated by signal {signal}\n");
        err.write_all(err_style.red(&line).as_bytes())?;
    } else if let Some(JsonValue::Integer(code)) = fields.get("exit_code")
        && *code != 0
    {
        let line = format!("koru: shell exited with status {code}\n");
        err.write_all(err_style.red(&line).as_bytes())?;
    }
    err.flush()
}

fn render_error(
    fields: &std::collections::BTreeMap<String, JsonValue>,
    err: &mut dyn Write,
    err_style: Style,
) -> io::Result<()> {
    let JsonValue::Object(error) = fields.get("error").expect("checked error envelope") else {
        return Ok(());
    };
    let code = match error.get("code") {
        Some(JsonValue::String(code)) => code.as_str(),
        _ => "error",
    };
    let message = match error.get("message") {
        Some(JsonValue::String(message)) => message.as_str(),
        _ => "the workflow could not complete the requested action",
    };
    let line = format!("koru [{code}]: {}\n", escape_text(message));
    err.write_all(err_style.red(&line).as_bytes())?;
    err.flush()
}

fn render_commit(
    fields: &std::collections::BTreeMap<String, JsonValue>,
    out: &mut dyn Write,
    style: Style,
) -> io::Result<()> {
    let status = string_field(fields, "status").unwrap_or("unknown");
    let changes = match fields.get("change_count") {
        Some(JsonValue::Integer(count)) => count.to_string(),
        _ => "?".to_owned(),
    };
    writeln!(out, "{} {status}", style.bold("commit plan:"))?;
    writeln!(out, "  changes: {changes}")?;
    if let Some(snapshot) = string_field(fields, "snapshot_id") {
        writeln!(out, "  snapshot: {snapshot}")?;
    }
    if let Some(plan) = string_field(fields, "plan_id") {
        writeln!(out, "  plan: {plan}")?;
    }
    if let Some(JsonValue::Array(groups)) = fields.get("groups") {
        for (index, group) in groups.iter().enumerate() {
            let JsonValue::Object(group) = group else {
                continue;
            };
            let message = string_field(group, "message").unwrap_or("(no message)");
            writeln!(out, "  {}. {}", index + 1, style.bold(message))?;
            if let Some(JsonValue::Array(ids)) = group.get("changes") {
                let ids = ids
                    .iter()
                    .filter_map(|id| match id {
                        JsonValue::String(id) => Some(id.as_str()),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                writeln!(out, "     changes: {ids}")?;
            }
            if let Some(rationale) = string_field(group, "rationale")
                && !rationale.is_empty()
            {
                writeln!(out, "     rationale: {}", escape_text(rationale))?;
            }
        }
    }
    out.flush()
}

fn string_field<'a>(
    fields: &'a std::collections::BTreeMap<String, JsonValue>,
    name: &str,
) -> Option<&'a str> {
    match fields.get(name) {
        Some(JsonValue::String(value)) => Some(value.as_str()),
        _ => None,
    }
}

fn is_true(value: Option<&JsonValue>) -> bool {
    matches!(value, Some(JsonValue::Bool(true)))
}

fn write_json(
    value: &JsonValue,
    depth: usize,
    out: &mut dyn Write,
    style: Style,
) -> io::Result<()> {
    match value {
        JsonValue::Object(fields) if !fields.is_empty() => {
            writeln!(out, "{{")?;
            for (index, (key, value)) in fields.iter().enumerate() {
                indent(out, depth + 1)?;
                let key = emit(&JsonValue::String(key.clone()));
                write!(out, "{}: ", style.cyan(&key))?;
                write_json(value, depth + 1, out, style)?;
                if index + 1 < fields.len() {
                    out.write_all(b",")?;
                }
                out.write_all(b"\n")?;
            }
            indent(out, depth)?;
            out.write_all(b"}")
        }
        JsonValue::Array(items) if !items.is_empty() => {
            writeln!(out, "[")?;
            for (index, item) in items.iter().enumerate() {
                indent(out, depth + 1)?;
                write_json(item, depth + 1, out, style)?;
                if index + 1 < items.len() {
                    out.write_all(b",")?;
                }
                out.write_all(b"\n")?;
            }
            indent(out, depth)?;
            out.write_all(b"]")
        }
        scalar => {
            let text = emit(scalar);
            let colored = match scalar {
                JsonValue::String(_) => style.green(&text),
                JsonValue::Integer(_) | JsonValue::Number(_) => style.yellow(&text),
                JsonValue::Bool(_) | JsonValue::Null => style.dim(&text),
                JsonValue::Object(_) | JsonValue::Array(_) => text,
            };
            out.write_all(colored.as_bytes())
        }
    }
}

fn indent(out: &mut dyn Write, depth: usize) -> io::Result<()> {
    for _ in 0..depth {
        out.write_all(b"  ")?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{escape_text, render_to};
    use crate::json::{JsonValue, parse};
    use crate::terminal::style::Style;

    fn value(text: &str) -> JsonValue {
        parse(text.as_bytes(), &Default::default()).unwrap()
    }

    fn render(text: &str) -> (String, String) {
        let mut out = Vec::new();
        let mut err = Vec::new();
        render_to(
            &value(text),
            &mut out,
            &mut err,
            Style::plain(),
            Style::plain(),
        )
        .unwrap();
        (
            String::from_utf8(out).unwrap(),
            String::from_utf8(err).unwrap(),
        )
    }

    #[test]
    fn a_shell_result_streams_output_instead_of_json() {
        let (out, err) = render(
            r#"{"ok":true,"exit_code":0,"signal":null,"stdout":"line one\nline two\n","stderr":"warn\n","stdout_truncated":false,"stderr_truncated":false}"#,
        );
        assert_eq!(out, "line one\nline two\n");
        assert_eq!(err, "warn\n");
    }

    #[test]
    fn a_failing_shell_result_reports_its_status() {
        let (out, err) = render(
            r#"{"ok":false,"exit_code":2,"signal":null,"stdout":"partial","stderr":"","stdout_truncated":false,"stderr_truncated":true}"#,
        );
        assert_eq!(out, "partial");
        assert!(err.contains("output truncated"), "{err}");
        assert!(err.contains("status 2"), "{err}");
    }

    #[test]
    fn an_error_envelope_is_reported_without_json() {
        let (out, err) =
            render(r#"{"ok":false,"error":{"code":"permission_denied","message":"denied"}}"#);
        assert!(out.is_empty());
        assert!(err.contains("permission_denied"), "{err}");
        assert!(err.contains("denied"), "{err}");
    }

    #[test]
    fn a_commit_plan_is_rendered_as_a_summary() {
        let (out, _) = render(
            r#"{"status":"plan_only","snapshot_id":"snap","plan_id":"plan","change_count":2,"groups":[{"changes":["a","b"],"message":"feat: group","rationale":"why"}]}"#,
        );
        assert!(out.contains("commit plan: plan_only"), "{out}");
        assert!(out.contains("feat: group"), "{out}");
        assert!(out.contains("a, b"), "{out}");
    }

    #[test]
    fn an_unavailable_commit_reports_its_error() {
        let (out, err) = render(
            r#"{"status":"unavailable","error":{"code":"permission_denied","message":"no"}}"#,
        );
        assert!(out.is_empty());
        assert!(err.contains("permission_denied"), "{err}");
    }

    #[test]
    fn a_shell_string_result_keeps_line_breaks() {
        let (out, err) = render(r#""hello\nworld\n""#);
        assert_eq!(out, "hello\nworld\n");
        assert!(err.is_empty());
    }

    #[test]
    fn unknown_objects_fall_back_to_indented_json() {
        let (out, _) = render(r#"{"a":1,"b":["x"]}"#);
        assert!(out.contains("\"a\": 1"), "{out}");
        assert!(out.contains("\"x\""), "{out}");
    }

    #[test]
    fn control_characters_are_escaped() {
        assert_eq!(escape_text("a\u{1b}[31mb"), "a\\u{1b}[31mb");
    }

    #[test]
    fn shell_results_propagate_their_exit_status() {
        let shell = |status: &str| {
            value(&format!(
                r#"{{"stdout":"","stderr":"","exit_code":{status},"signal":null}}"#
            ))
        };
        assert_eq!(super::exit_code(&shell("0")), 0);
        assert_eq!(super::exit_code(&shell("3")), 3);
        assert_eq!(
            super::exit_code(&value(
                r#"{"stdout":"","stderr":"","exit_code":null,"signal":9}"#
            )),
            137
        );
        assert_eq!(super::exit_code(&value(r#""plain text""#)), 0);
    }
}
