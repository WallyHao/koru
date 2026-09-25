//! Environment credentials and value redaction.
//!
//! Provider keys are read from the process environment through the [`Environment`]
//! trait so tests never mutate process globals. Credentials never enter
//! `config.toml`, never reach Lua, and are removed from error and diagnostic text
//! by [`redact`]. A [`Credentials`] value deliberately prints only whether each
//! key is set.
use crate::error::{ErrorCode, KoruError, Result};
use std::collections::BTreeMap;

/// Environment variable holding the DeepSeek API key.
pub const DEEPSEEK_API_KEY: &str = "DEEPSEEK_API_KEY";
/// Environment variable holding the OpenCode (Zen and Go) API key.
pub const OPENCODE_API_KEY: &str = "OPENCODE_API_KEY";

/// All credential variables Koru understands.
pub const CREDENTIAL_VARIABLES: [&str; 2] = [DEEPSEEK_API_KEY, OPENCODE_API_KEY];

/// A read-only view of environment variables.
pub trait Environment {
    /// Return the variable's value when it is set and valid UTF-8.
    fn get(&self, name: &str) -> Option<String>;
}

/// The real process environment.
#[derive(Debug, Default, Clone, Copy)]
pub struct ProcessEnvironment;
impl Environment for ProcessEnvironment {
    fn get(&self, name: &str) -> Option<String> {
        std::env::var(name).ok()
    }
}

/// A fixed environment used by tests.
#[derive(Debug, Default, Clone)]
pub struct MapEnvironment(BTreeMap<String, String>);
impl MapEnvironment {
    /// Build a map environment from name/value pairs.
    pub fn new(pairs: impl IntoIterator<Item = (impl Into<String>, impl Into<String>)>) -> Self {
        Self(
            pairs
                .into_iter()
                .map(|(name, value)| (name.into(), value.into()))
                .collect(),
        )
    }
}
impl Environment for MapEnvironment {
    fn get(&self, name: &str) -> Option<String> {
        self.0.get(name).cloned()
    }
}

/// The resolved provider credentials.
#[derive(Clone, Default)]
pub struct Credentials {
    values: BTreeMap<String, String>,
}
impl Credentials {
    /// Read every known credential variable from `env`.
    pub fn from_environment(env: &dyn Environment) -> Self {
        let mut values = BTreeMap::new();
        for variable in CREDENTIAL_VARIABLES {
            if let Some(value) = env.get(variable) {
                values.insert(variable.to_owned(), value);
            }
        }
        Self { values }
    }

    /// Return a required credential or an actionable error naming its variable.
    pub fn require(&self, variable: &str) -> Result<&str> {
        match self.values.get(variable) {
            Some(value) if !value.is_empty() => Ok(value),
            _ => Err(KoruError::new(
                ErrorCode::Validation,
                format!("{variable} is not set; export it to use this provider"),
            )),
        }
    }

    /// Whether a credential is present and non-empty.
    pub fn is_set(&self, variable: &str) -> bool {
        self.values
            .get(variable)
            .is_some_and(|value| !value.is_empty())
    }

    /// Remove every known credential value from `text`.
    pub fn redact(&self, text: &str) -> String {
        redact(self.values.values().map(String::as_str), text)
    }
}
impl std::fmt::Debug for Credentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut set = Vec::new();
        for variable in CREDENTIAL_VARIABLES {
            if self.is_set(variable) {
                set.push(variable);
            }
        }
        f.debug_struct("Credentials").field("set", &set).finish()
    }
}

/// Replace every non-empty secret in `secrets` with `[redacted]`.
pub fn redact<'a>(secrets: impl IntoIterator<Item = &'a str>, text: &str) -> String {
    let mut out = text.to_owned();
    for secret in secrets {
        if !secret.is_empty() {
            out = out.replace(secret, "[redacted]");
        }
    }
    out
}
