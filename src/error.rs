//! Stable error categories shared by the CLI and future adapters.
use std::path::PathBuf;

/// Koru's stable machine-readable failure categories.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorCode {
    /// A user-supplied value or source violates a contract.
    Validation,
    /// An input or operation exceeded a configured limit.
    BudgetExhausted,
    /// Access was denied by host-owned policy.
    PermissionDenied,
    /// A requested feature is not yet supported.
    UnsupportedCapability,
    /// State changed after a snapshot or authorization.
    StateConflict,
    /// The execution context can no longer accept effects.
    Cancelled,
    /// The execution deadline expired.
    Timeout,
    /// A filesystem or other I/O operation failed.
    Io,
}
impl ErrorCode {
    /// Stable code for diagnostics and automation.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Validation => "validation",
            Self::BudgetExhausted => "budget_exhausted",
            Self::PermissionDenied => "permission_denied",
            Self::UnsupportedCapability => "unsupported_capability",
            Self::StateConflict => "state_conflict",
            Self::Cancelled => "cancelled",
            Self::Timeout => "timeout",
            Self::Io => "io",
        }
    }
}

/// A contextual error with an optional original I/O cause.
#[derive(Debug, thiserror::Error)]
#[error("{code:?}: {message}")]
pub struct KoruError {
    code: ErrorCode,
    message: String,
    #[source]
    source: Option<std::io::Error>,
}
impl KoruError {
    /// Build an error without an underlying I/O cause.
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            source: None,
        }
    }
    /// Build an error retaining its filesystem cause.
    pub fn io(message: impl Into<String>, source: std::io::Error) -> Self {
        Self {
            code: ErrorCode::Io,
            message: message.into(),
            source: Some(source),
        }
    }
    /// The stable category of this error.
    pub const fn code(&self) -> ErrorCode {
        self.code
    }
    /// The human-readable message, excluding its code.
    pub fn message(&self) -> &str {
        &self.message
    }
    /// Associate a path with an I/O error without discarding its cause.
    pub fn at_path(path: PathBuf, source: std::io::Error) -> Self {
        Self::io(format!("cannot access {}", path.display()), source)
    }
}
/// Koru's typed result.
pub type Result<T> = std::result::Result<T, KoruError>;
