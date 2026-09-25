//! Validate command and module names before using them as path components.
use crate::error::{ErrorCode, KoruError, Result};

const BUILTINS: &[&str] = &[
    "model", "variant", "check", "recover", "help", "list", "inspect",
];

/// Validate an installed command's portable name.
pub fn command(name: &str) -> Result<()> {
    if !identifier(name) || BUILTINS.contains(&name) {
        return Err(KoruError::new(
            ErrorCode::Validation,
            format!("invalid or reserved command name {name:?}"),
        ));
    }
    Ok(())
}
/// Validate a pure-Lua dotted module name.
pub fn module(name: &str) -> Result<()> {
    if name.is_empty() || !name.split('.').all(identifier) {
        return Err(KoruError::new(
            ErrorCode::Validation,
            format!("invalid module name {name:?}"),
        ));
    }
    Ok(())
}
/// Validate a portable lowercase identifier shared by commands, modules, and arguments.
pub(crate) fn identifier(value: &str) -> bool {
    let mut chars = value.chars();
    matches!(chars.next(), Some('a'..='z'))
        && chars.all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '_')
        && value.len() <= 64
}
