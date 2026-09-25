//! Koru-owned JSON Schema subset used to validate tool contracts.
//!
//! Only the documented v1 subset is accepted: `type`, `properties`, `required`,
//! `additionalProperties`, `items`, `minItems`, `maxItems`, `minLength`,
//! `maxLength`, `minimum`, `maximum`, `enum`, and the `description` annotation.
//! Every other keyword and any remote reference is rejected, so a provider never
//! silently ignores a constraint. Compilation (`compile`) and checking
//! (`validate`) live in submodules; this root holds the data model and shared
//! helpers.
mod compile;
mod read;
mod validate;

use crate::{
    error::{ErrorCode, KoruError},
    json::JsonValue,
};
use std::collections::{BTreeMap, BTreeSet};

/// A JSON type name supported by the Koru schema subset.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum JsonType {
    /// A JSON object.
    Object,
    /// A JSON array.
    Array,
    /// A JSON string.
    String,
    /// A JSON boolean.
    Boolean,
    /// A JSON integer within the portable range.
    Integer,
    /// A finite JSON number.
    Number,
    /// JSON null.
    Null,
}
impl JsonType {
    /// Stable name used in error messages.
    pub fn name(self) -> &'static str {
        match self {
            Self::Object => "object",
            Self::Array => "array",
            Self::String => "string",
            Self::Boolean => "boolean",
            Self::Integer => "integer",
            Self::Number => "number",
            Self::Null => "null",
        }
    }
    fn from_name(name: &str) -> Option<Self> {
        Some(match name {
            "object" => Self::Object,
            "array" => Self::Array,
            "string" => Self::String,
            "boolean" => Self::Boolean,
            "integer" => Self::Integer,
            "number" => Self::Number,
            "null" => Self::Null,
            _ => return None,
        })
    }
}

/// Validation keywords the Koru schema subset understands.
pub const SUPPORTED_KEYWORDS: [&str; 12] = [
    "type",
    "enum",
    "minimum",
    "maximum",
    "minLength",
    "maxLength",
    "items",
    "minItems",
    "maxItems",
    "properties",
    "required",
    "additionalProperties",
];

/// A compiled, bounded JSON schema.
#[derive(Debug, Clone, PartialEq)]
pub struct JsonSchema {
    types: Option<BTreeSet<JsonType>>,
    enum_values: Option<Vec<JsonValue>>,
    minimum: Option<f64>,
    maximum: Option<f64>,
    min_length: Option<u64>,
    max_length: Option<u64>,
    items: Option<Box<JsonSchema>>,
    min_items: Option<u64>,
    max_items: Option<u64>,
    properties: BTreeMap<String, JsonSchema>,
    required: BTreeSet<String>,
    additional_properties: bool,
    keywords: BTreeSet<&'static str>,
    document: JsonValue,
}
impl JsonSchema {
    /// Compile a bounded schema document; unsupported keywords are rejected.
    pub fn compile(
        document: &JsonValue,
        limits: &crate::json::JsonLimits,
    ) -> crate::error::Result<Self> {
        Self::compile_at(document, limits, 0, "")
    }

    /// Validate one value, returning a path-qualified error on mismatch.
    pub fn validate(&self, value: &JsonValue) -> crate::error::Result<()> {
        self.validate_at(value, "")
    }

    /// Validation keywords used anywhere in this schema.
    pub fn keywords(&self) -> &BTreeSet<&'static str> {
        &self.keywords
    }

    /// The original bounded schema document, for protocol payloads.
    pub fn document(&self) -> &JsonValue {
        &self.document
    }

    /// Whether this schema accepts objects at its root and only objects.
    ///
    /// Tool argument schemas must describe an object, so a schema with no type
    /// or a union that also accepts non-objects is rejected by the caller.
    pub fn is_object_root(&self) -> bool {
        matches!(&self.types, Some(types)
            if types.len() == 1 && types.contains(&JsonType::Object))
    }
}

fn child(path: &str, segment: &str) -> String {
    let segment = segment.replace('~', "~0").replace('/', "~1");
    format!("{path}/{segment}")
}

fn schema_error(path: &str, detail: impl Into<String>) -> KoruError {
    let detail = detail.into();
    let message = if path.is_empty() {
        detail
    } else {
        format!("{path}: {detail}")
    };
    KoruError::new(ErrorCode::Validation, message)
}
