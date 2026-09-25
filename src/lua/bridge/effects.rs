//! Parse shell requests and complete them through the host-owned effect gate.
use super::{Pending, PendingRequest};
use crate::{
    effects::{ProcessResult, execute_process},
    error::{KoruError, Result},
    lua::{ApprovalProvider, error},
    permissions::{Broker, Policy, PreparedAction},
    runtime::ExecutionContext,
};
use mlua::{Lua, Table, Value};
use std::{cell::Cell, path::PathBuf, rc::Rc};

const MAX_SCRIPT_BYTES: usize = 64 * 1024;
const MAX_CWD_BYTES: usize = 4096;
const SHELL: &str = "/bin/sh";

/// Owned request left by a suspended Lua shell call.
pub(super) struct ShellRequest {
    script: String,
    cwd: PathBuf,
    explanation: String,
}

pub(super) fn build_shell(lua: &Lua, in_tool: Rc<Cell<bool>>, pending: Pending) -> Result<Table> {
    let shell = lua
        .create_table()
        .map_err(|cause| error::invalid(format!("cannot build koru.shell: {cause}")))?;
    let script = lua
        .create_async_function(move |lua, (script_text, options): (String, Table)| {
            let flag = Rc::clone(&in_tool);
            let slot = Rc::clone(&pending);
            async move {
                if flag.get() {
                    return Err(mlua::Error::RuntimeError(
                        "host effects from a tool callback are not supported".to_owned(),
                    ));
                }
                let request = read_options(script_text, &options)?;
                *slot.borrow_mut() = Some(PendingRequest::Shell(request));
                let response: Table = lua.yield_with(Value::Nil).await?;
                if response.get::<bool>("ok")? {
                    Ok((Some(response.get::<Table>("result")?), None::<Table>))
                } else {
                    Ok((None::<Table>, Some(response.get::<Table>("error")?)))
                }
            }
        })
        .map_err(|cause| error::invalid(format!("cannot build koru.shell.script: {cause}")))?;
    shell
        .set("script", script)
        .map_err(|cause| error::invalid(format!("cannot build koru.shell: {cause}")))?;
    Ok(shell)
}

pub(super) fn complete_shell(
    lua: &Lua,
    request: ShellRequest,
    context: &ExecutionContext,
    approval: &mut dyn ApprovalProvider,
) -> mlua::Result<Value> {
    let outcome = (|| {
        if !cfg!(target_os = "linux") {
            return Err(KoruError::new(
                crate::error::ErrorCode::UnsupportedCapability,
                "approved shell execution is currently supported only on Linux",
            ));
        }
        context.ensure_active(std::time::Instant::now())?;
        let action = PreparedAction::shell(context, SHELL.into(), request.script, request.cwd)?;
        if !request.explanation.is_empty() {
            let escaped: String = request
                .explanation
                .chars()
                .flat_map(char::escape_default)
                .collect();
            let style = crate::terminal::Style::stderr();
            eprintln!("{} {escaped}", style.bold("Plan:"));
        }
        let decision = approval.decide(&action, context.deadline())?;
        let policy = Policy::new(1)?;
        let approved = Broker::authorize(
            action,
            context,
            &policy,
            decision,
            std::time::Instant::now(),
        )?;
        execute_process(approved, context)
    })();
    response(lua, outcome)
}

fn read_options(script: String, options: &Table) -> mlua::Result<ShellRequest> {
    if script.len() > MAX_SCRIPT_BYTES {
        return Err(mlua::Error::RuntimeError("`script` is too long".into()));
    }
    let mut cwd = None;
    let mut explanation = None;
    for pair in options.pairs::<String, Value>() {
        let (key, value) = pair?;
        match key.as_str() {
            "cwd" => cwd = Some(PathBuf::from(bounded_text(&value, "cwd", MAX_CWD_BYTES)?)),
            "explanation" => explanation = Some(bounded_text(&value, "explanation", 1024)?),
            other => {
                return Err(mlua::Error::RuntimeError(format!(
                    "unknown shell.script option {other:?}"
                )));
            }
        }
    }
    Ok(ShellRequest {
        script,
        cwd: cwd.ok_or_else(|| mlua::Error::RuntimeError("shell.script needs a `cwd`".into()))?,
        explanation: explanation.unwrap_or_default(),
    })
}

fn bounded_text(value: &Value, field: &str, limit: usize) -> mlua::Result<String> {
    let value = value
        .as_string()
        .ok_or_else(|| mlua::Error::RuntimeError(format!("`{field}` must be a string")))?;
    let value = value
        .to_str()
        .map_err(|_| mlua::Error::RuntimeError(format!("`{field}` must be valid UTF-8")))?;
    if value.len() > limit {
        return Err(mlua::Error::RuntimeError(format!("`{field}` is too long")));
    }
    Ok(value.as_ref().to_owned())
}

fn response(lua: &Lua, outcome: Result<ProcessResult>) -> mlua::Result<Value> {
    let envelope = lua.create_table()?;
    match outcome {
        Ok(result) => {
            envelope.set("ok", true)?;
            let value = lua.create_table()?;
            value.set("exit_code", result.exit_code)?;
            value.set("signal", result.signal)?;
            value.set("stdout", String::from_utf8_lossy(&result.stdout).as_ref())?;
            value.set("stderr", String::from_utf8_lossy(&result.stderr).as_ref())?;
            value.set("stdout_truncated", result.stdout_truncated)?;
            value.set("stderr_truncated", result.stderr_truncated)?;
            envelope.set("result", value)?;
        }
        Err(cause) => {
            envelope.set("ok", false)?;
            envelope.set("error", error_table(lua, &cause)?)?;
        }
    }
    Ok(Value::Table(envelope))
}

fn error_table(lua: &Lua, cause: &KoruError) -> mlua::Result<Table> {
    let error = lua.create_table()?;
    error.set("code", cause.code().as_str())?;
    error.set("message", cause.message())?;
    Ok(error)
}
