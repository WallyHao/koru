//! Koru-owned JSON value model with explicit, bounded conversion rules.
use crate::error::{ErrorCode, KoruError, Result};
use std::collections::BTreeMap;

/// Largest integer that is exact in the JSON/JavaScript number range.
pub const MAX_SAFE_INTEGER: i64 = 9_007_199_254_740_991;

/// Hard ceilings applied while converting one JSON value.
#[derive(Debug, Clone, Copy)]
pub struct JsonLimits {
    /// Maximum nesting depth, with a scalar at depth zero.
    pub max_depth: usize,
    /// Maximum total number of array elements and object entries.
    pub max_elements: usize,
    /// Maximum total string and key bytes.
    pub max_bytes: usize,
    /// Maximum bytes in one string value.
    pub max_string_bytes: usize,
    /// Maximum bytes in one object key.
    pub max_key_bytes: usize,
}
impl Default for JsonLimits {
    fn default() -> Self {
        Self {
            max_depth: 32,
            max_elements: 100_000,
            max_bytes: 8 * 1024 * 1024,
            max_string_bytes: 1024 * 1024,
            max_key_bytes: 1024,
        }
    }
}
impl JsonLimits {
    /// Reject values nested deeper than the limit.
    pub fn check_depth(&self, depth: usize) -> Result<()> {
        if depth > self.max_depth {
            return Err(limit("JSON nesting depth"));
        }
        Ok(())
    }
    /// Reject values with more than the allowed elements.
    pub fn check_elements(&self, count: usize) -> Result<()> {
        if count > self.max_elements {
            return Err(limit("JSON element count"));
        }
        Ok(())
    }
    /// Reject values with more than the allowed total bytes.
    pub fn check_bytes(&self, total: usize) -> Result<()> {
        if total > self.max_bytes {
            return Err(limit("JSON byte size"));
        }
        Ok(())
    }
    /// Reject one string longer than the allowed size.
    pub fn check_string_bytes(&self, bytes: usize) -> Result<()> {
        if bytes > self.max_string_bytes {
            return Err(limit("JSON string length"));
        }
        Ok(())
    }
    /// Reject one object key longer than the allowed size.
    pub fn check_key_bytes(&self, bytes: usize) -> Result<()> {
        if bytes > self.max_key_bytes {
            return Err(limit("JSON key length"));
        }
        Ok(())
    }
}

/// A JSON value with an unambiguous representation of null and empty containers.
#[derive(Debug, Clone, PartialEq)]
pub enum JsonValue {
    /// JSON null, distinct from an absent Lua value.
    Null,
    /// A boolean.
    Bool(bool),
    /// An exact integer within [`MAX_SAFE_INTEGER`].
    Integer(i64),
    /// A finite number.
    Number(f64),
    /// A UTF-8 string.
    String(String),
    /// A contiguous array.
    Array(Vec<JsonValue>),
    /// A string-keyed object.
    Object(BTreeMap<String, JsonValue>),
}
impl JsonValue {
    /// A short name for diagnostics.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Null => "null",
            Self::Bool(_) => "boolean",
            Self::Integer(_) => "integer",
            Self::Number(_) => "number",
            Self::String(_) => "string",
            Self::Array(_) => "array",
            Self::Object(_) => "object",
        }
    }
}

fn limit(kind: &str) -> KoruError {
    KoruError::new(ErrorCode::BudgetExhausted, format!("{kind} limit exceeded"))
}
