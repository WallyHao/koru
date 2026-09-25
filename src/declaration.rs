//! Koru-owned API version 1 command declaration contract and pure validation.
//!
//! This module has no Lua dependency. The Lua adapter converts evaluated values
//! into these types; `CommandDeclaration::new` is the single validator shared by
//! `koru check` and the future workflow runtime.
use crate::{
    error::{ErrorCode, KoruError, Result},
    json::{JsonLimits, JsonValue},
    schema::JsonSchema,
    source::names,
};
use std::collections::{BTreeMap, BTreeSet};

pub use crate::json::MAX_SAFE_INTEGER;

/// Maximum bytes in a declaration description.
pub const MAX_DESCRIPTION_BYTES: usize = 1024;
/// Maximum number of declared arguments.
pub const MAX_ARGUMENTS: usize = 32;
/// Maximum number of declared tools.
pub const MAX_TOOLS: usize = 32;
/// Maximum values in one enum argument.
pub const MAX_ENUM_VALUES: usize = 64;
/// Maximum bytes in argument help text.
pub const MAX_HELP_BYTES: usize = 1024;
/// Maximum bytes in a string default or enum value.
pub const MAX_VALUE_BYTES: usize = 4096;

/// A validated API version 1 command declaration.
#[derive(Debug, Clone, PartialEq)]
pub struct CommandDeclaration {
    api_version: u32,
    description: String,
    arguments: Vec<Argument>,
    capabilities: Capabilities,
    tools: Vec<ToolDeclaration>,
}
impl CommandDeclaration {
    /// Validate and construct a declaration from already-parsed parts.
    pub fn new(
        api_version: u32,
        description: impl Into<String>,
        arguments: Vec<Argument>,
        capabilities: Capabilities,
        tools: Vec<ToolDeclaration>,
    ) -> Result<Self> {
        if api_version != crate::API_VERSION {
            return Err(KoruError::new(
                ErrorCode::UnsupportedCapability,
                format!("unsupported declaration API version {api_version}"),
            ));
        }
        let description = description.into();
        validate_text("description", &description, MAX_DESCRIPTION_BYTES, true)?;
        if arguments.len() > MAX_ARGUMENTS {
            return Err(KoruError::new(
                ErrorCode::Validation,
                format!("a command may declare at most {MAX_ARGUMENTS} arguments"),
            ));
        }
        let mut names = BTreeSet::new();
        for argument in &arguments {
            argument.validate()?;
            if !names.insert(argument.name.as_str()) {
                return Err(KoruError::new(
                    ErrorCode::Validation,
                    format!("duplicate argument name {:?}", argument.name),
                ));
            }
        }
        if tools.len() > MAX_TOOLS {
            return Err(KoruError::new(
                ErrorCode::Validation,
                format!("a command may declare at most {MAX_TOOLS} tools"),
            ));
        }
        let mut tool_names = BTreeSet::new();
        for tool in &tools {
            if !tool_names.insert(tool.name()) {
                return Err(KoruError::new(
                    ErrorCode::Validation,
                    format!("duplicate tool name {:?}", tool.name()),
                ));
            }
        }
        Ok(Self {
            api_version,
            description,
            arguments,
            capabilities,
            tools,
        })
    }
    /// The declared API version, always [`crate::API_VERSION`] when valid.
    pub fn api_version(&self) -> u32 {
        self.api_version
    }
    /// One-line human-readable summary.
    pub fn description(&self) -> &str {
        &self.description
    }
    /// Declared arguments in order.
    pub fn arguments(&self) -> &[Argument] {
        &self.arguments
    }
    /// Capabilities requested by the script; never a grant.
    pub fn capabilities(&self) -> Capabilities {
        self.capabilities
    }
    /// Declared tools in order; callbacks are retained by the Lua adapter.
    pub fn tools(&self) -> &[ToolDeclaration] {
        &self.tools
    }

    /// Parse positional CLI values before constructing a provider or running Lua.
    pub fn parse_args(&self, supplied: &[String]) -> Result<JsonValue> {
        if supplied.len() > self.arguments.len() {
            return Err(KoruError::new(
                ErrorCode::Validation,
                format!(
                    "expected at most {} command arguments",
                    self.arguments.len()
                ),
            ));
        }
        let mut parsed = BTreeMap::new();
        for (index, argument) in self.arguments.iter().enumerate() {
            let value = match supplied.get(index) {
                Some(text) => argument.parse(text)?,
                None if argument.required => {
                    return Err(KoruError::new(
                        ErrorCode::Validation,
                        format!("missing required argument {:?}", argument.name),
                    ));
                }
                None => argument
                    .default
                    .as_ref()
                    .map_or(JsonValue::Null, ArgumentValue::json),
            };
            parsed.insert(argument.name.clone(), value);
        }
        Ok(JsonValue::Object(parsed))
    }
}

/// A validated tool declaration with compiled schemas; the callback is retained
/// by the Lua adapter.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolDeclaration {
    name: String,
    description: String,
    parameters: JsonSchema,
    result: Option<JsonSchema>,
}
impl ToolDeclaration {
    /// Validate tool metadata and compile its argument and result schemas.
    ///
    /// The argument schema must describe an object at its root; the optional
    /// result schema may describe any supported JSON value.
    pub fn new(
        name: impl Into<String>,
        description: impl Into<String>,
        parameters: JsonValue,
        result: Option<JsonValue>,
    ) -> Result<Self> {
        let name = name.into();
        let description = description.into();
        if !names::identifier(&name) {
            return Err(KoruError::new(
                ErrorCode::Validation,
                format!("invalid tool name {name:?}"),
            ));
        }
        validate_text(
            "tool description",
            &description,
            MAX_DESCRIPTION_BYTES,
            true,
        )?;
        let limits = JsonLimits::default();
        let parameters = JsonSchema::compile(&parameters, &limits)?;
        if !parameters.is_object_root() {
            return Err(KoruError::new(
                ErrorCode::Validation,
                format!("tool {name:?} parameters must describe an object"),
            ));
        }
        let result = match result {
            Some(document) => Some(JsonSchema::compile(&document, &limits)?),
            None => None,
        };
        Ok(Self {
            name,
            description,
            parameters,
            result,
        })
    }
    /// Unique portable tool name.
    pub fn name(&self) -> &str {
        &self.name
    }
    /// Human-readable description.
    pub fn description(&self) -> &str {
        &self.description
    }
    /// Compiled schema for the tool arguments.
    pub fn parameters(&self) -> &JsonSchema {
        &self.parameters
    }
    /// Compiled schema for the tool result, when declared.
    pub fn result(&self) -> Option<&JsonSchema> {
        self.result.as_ref()
    }
}

/// Capabilities a script requests; the policy resolver alone produces grants.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Capabilities {
    /// The script asks to invoke direct processes.
    pub direct_processes: bool,
}

/// One declared command argument and its schema.
#[derive(Debug, Clone, PartialEq)]
pub struct Argument {
    /// Portable lowercase name, unique within the declaration.
    pub name: String,
    /// Expected value type.
    pub kind: ArgumentType,
    /// Whether the caller must supply the argument.
    pub required: bool,
    /// Default value used when the argument is omitted.
    pub default: Option<ArgumentValue>,
    /// Short help text for the argument.
    pub help: Option<String>,
    /// Inclusive lower bound for numeric arguments.
    pub min: Option<f64>,
    /// Inclusive upper bound for numeric arguments.
    pub max: Option<f64>,
    /// Maximum byte length for string arguments.
    pub max_len: Option<u64>,
}
impl Argument {
    fn parse(&self, text: &str) -> Result<JsonValue> {
        let value = match &self.kind {
            ArgumentType::String => JsonValue::String(text.to_owned()),
            ArgumentType::Enum(values) if values.iter().any(|value| value == text) => {
                JsonValue::String(text.to_owned())
            }
            ArgumentType::Integer => {
                let number = text.parse::<i64>().map_err(|_| self.bad_value())?;
                if !(-MAX_SAFE_INTEGER..=MAX_SAFE_INTEGER).contains(&number) {
                    return Err(self.bad_value());
                }
                JsonValue::Integer(number)
            }
            ArgumentType::Number => {
                let number = text.parse::<f64>().map_err(|_| self.bad_value())?;
                if !number.is_finite() {
                    return Err(self.bad_value());
                }
                JsonValue::Number(number)
            }
            ArgumentType::Boolean => match text {
                "true" => JsonValue::Bool(true),
                "false" => JsonValue::Bool(false),
                _ => return Err(self.bad_value()),
            },
            ArgumentType::Enum(_) => return Err(self.bad_value()),
        };
        if let Some(max_len) = self.max_len
            && text.len() as u64 > max_len
        {
            return Err(self.bad_value());
        }
        let numeric = match value {
            JsonValue::Integer(number) => Some(number as f64),
            JsonValue::Number(number) => Some(number),
            _ => None,
        };
        if let Some(number) = numeric
            && (self.min.is_some_and(|min| number < min)
                || self.max.is_some_and(|max| number > max))
        {
            return Err(self.bad_value());
        }
        Ok(value)
    }
    fn bad_value(&self) -> KoruError {
        KoruError::new(
            ErrorCode::Validation,
            format!("invalid value for argument {:?}", self.name),
        )
    }
    fn validate(&self) -> Result<()> {
        if !names::identifier(&self.name) {
            return Err(KoruError::new(
                ErrorCode::Validation,
                format!("invalid argument name {:?}", self.name),
            ));
        }
        if let Some(help) = &self.help {
            validate_text("argument help", help, MAX_HELP_BYTES, false)?;
        }
        if self.required && self.default.is_some() {
            return Err(KoruError::new(
                ErrorCode::Validation,
                format!(
                    "argument {:?} cannot be required and have a default",
                    self.name
                ),
            ));
        }
        if let Some(default) = &self.default {
            default.check(&self.kind, &self.name)?;
        }
        let numeric = matches!(self.kind, ArgumentType::Integer | ArgumentType::Number);
        if (self.min.is_some() || self.max.is_some()) && !numeric {
            return Err(KoruError::new(
                ErrorCode::Validation,
                format!(
                    "argument {:?} has numeric bounds but is not numeric",
                    self.name
                ),
            ));
        }
        for bound in [self.min, self.max].into_iter().flatten() {
            if !bound.is_finite() {
                return Err(KoruError::new(
                    ErrorCode::Validation,
                    format!("argument {:?} has a nonfinite bound", self.name),
                ));
            }
        }
        if let (Some(min), Some(max)) = (self.min, self.max)
            && min > max
        {
            return Err(KoruError::new(
                ErrorCode::Validation,
                format!("argument {:?} has min greater than max", self.name),
            ));
        }
        if self.max_len.is_some() && !matches!(self.kind, ArgumentType::String) {
            return Err(KoruError::new(
                ErrorCode::Validation,
                format!("argument {:?} has max_len but is not a string", self.name),
            ));
        }
        if let ArgumentType::Enum(values) = &self.kind {
            if values.is_empty() || values.len() > MAX_ENUM_VALUES {
                return Err(KoruError::new(
                    ErrorCode::Validation,
                    format!(
                        "argument {:?} needs 1..={MAX_ENUM_VALUES} enum values",
                        self.name
                    ),
                ));
            }
            let mut seen = BTreeSet::new();
            for value in values {
                validate_text("enum value", value, MAX_VALUE_BYTES, false)?;
                if !seen.insert(value.as_str()) {
                    return Err(KoruError::new(
                        ErrorCode::Validation,
                        format!("argument {:?} has a duplicate enum value", self.name),
                    ));
                }
            }
        }
        Ok(())
    }
}

/// Supported argument value types; v1 has no arrays or objects.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArgumentType {
    /// A UTF-8 string.
    String,
    /// A portable integer within the exact JSON number range.
    Integer,
    /// A finite number.
    Number,
    /// A boolean.
    Boolean,
    /// One of a fixed set of strings.
    Enum(Vec<String>),
}

/// A literal default value for an argument.
#[derive(Debug, Clone, PartialEq)]
pub enum ArgumentValue {
    /// A string literal.
    String(String),
    /// An integer literal.
    Integer(i64),
    /// A finite number literal.
    Number(f64),
    /// A boolean literal.
    Boolean(bool),
}
impl ArgumentValue {
    fn json(&self) -> JsonValue {
        match self {
            Self::String(value) => JsonValue::String(value.clone()),
            Self::Integer(value) => JsonValue::Integer(*value),
            Self::Number(value) => JsonValue::Number(*value),
            Self::Boolean(value) => JsonValue::Bool(*value),
        }
    }
    fn check(&self, kind: &ArgumentType, name: &str) -> Result<()> {
        let valid = match (self, kind) {
            (Self::String(value), ArgumentType::String) => {
                validate_text("default", value, MAX_VALUE_BYTES, false).is_ok()
            }
            (Self::String(value), ArgumentType::Enum(values)) => {
                values.iter().any(|item| item == value)
            }
            (Self::Integer(value), ArgumentType::Integer) => {
                (-MAX_SAFE_INTEGER..=MAX_SAFE_INTEGER).contains(value)
            }
            (Self::Number(value), ArgumentType::Number) => value.is_finite(),
            (Self::Boolean(_), ArgumentType::Boolean) => true,
            _ => false,
        };
        if valid {
            Ok(())
        } else {
            Err(KoruError::new(
                ErrorCode::Validation,
                format!("default for argument {name:?} does not match its type"),
            ))
        }
    }
}

fn validate_text(field: &str, value: &str, max_bytes: usize, require_nonempty: bool) -> Result<()> {
    if require_nonempty && value.is_empty() {
        return Err(KoruError::new(
            ErrorCode::Validation,
            format!("{field} must not be empty"),
        ));
    }
    if value.len() > max_bytes {
        return Err(KoruError::new(
            ErrorCode::Validation,
            format!("{field} exceeds {max_bytes} bytes"),
        ));
    }
    if value.chars().any(char::is_control) {
        return Err(KoruError::new(
            ErrorCode::Validation,
            format!("{field} contains control characters"),
        ));
    }
    Ok(())
}
