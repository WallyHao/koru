//! Map mlua failures onto stable Koru error categories with bounded text.
use crate::{
    error::{ErrorCode, KoruError},
    runtime::{ExecutionContext, RunState},
};

const MAX_ERROR_BYTES: usize = 1024;

/// Build a validation error, escaping and bounding all message text.
pub(super) fn invalid(message: impl Into<String>) -> KoruError {
    KoruError::new(ErrorCode::Validation, escape(&message.into()))
}

/// Build an unsupported-capability error for a known but unimplemented feature.
pub(super) fn unsupported(message: impl Into<String>) -> KoruError {
    KoruError::new(ErrorCode::UnsupportedCapability, escape(&message.into()))
}

/// Build an error in a chosen category with escaped, bounded text.
pub(super) fn coded(code: ErrorCode, message: impl Into<String>) -> KoruError {
    KoruError::new(code, escape(&message.into()))
}

/// Classify an mlua failure, preferring the terminal execution state.
pub(super) fn map(context: &ExecutionContext, error: mlua::Error, what: &str) -> KoruError {
    if let Ok(state) = context.state() {
        let terminal = match state {
            RunState::Exhausted => Some((ErrorCode::BudgetExhausted, "execution budget exhausted")),
            RunState::Cancelled => Some((ErrorCode::Cancelled, "execution cancelled")),
            RunState::TimedOut => Some((ErrorCode::Timeout, "execution deadline exceeded")),
            RunState::Active => None,
        };
        if let Some((code, message)) = terminal {
            return KoruError::new(code, message);
        }
    }
    if matches!(error, mlua::Error::MemoryError(_)) {
        return KoruError::new(ErrorCode::BudgetExhausted, "Lua memory limit exceeded");
    }
    invalid(format!("{what}: {error}"))
}

fn escape(value: &str) -> String {
    let mut out = String::new();
    for ch in value.chars() {
        if out.len() >= MAX_ERROR_BYTES {
            break;
        }
        if ch.is_control() {
            out.extend(ch.escape_default());
        } else {
            out.push(ch);
        }
    }
    out
}
