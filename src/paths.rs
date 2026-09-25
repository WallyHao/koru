//! Resolve user-owned XDG paths once at the CLI composition root.
use crate::error::{ErrorCode, KoruError, Result};
use std::{env, path::PathBuf};

/// Immutable paths for Koru's persistent user data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserPaths {
    /// Location of configuration and command source files.
    pub config: PathBuf,
    /// Location of disposable model catalogs.
    pub cache: PathBuf,
    /// Location of durable journals.
    pub state: PathBuf,
}
impl UserPaths {
    /// Resolve XDG overrides, falling back to the standard home directories.
    pub fn from_environment() -> Result<Self> {
        let home = env::var_os("HOME")
            .map(PathBuf::from)
            .ok_or_else(|| KoruError::new(ErrorCode::Validation, "HOME must be set"))?;
        fn xdg(key: &str, home: &std::path::Path, fallback: &str) -> Result<PathBuf> {
            match env::var_os(key) {
                Some(value) if !value.is_empty() => {
                    let path = PathBuf::from(value);
                    if !path.is_absolute() {
                        return Err(KoruError::new(
                            ErrorCode::Validation,
                            format!("{key} must be an absolute path"),
                        ));
                    }
                    Ok(path.join("koru"))
                }
                _ => Ok(home.join(fallback).join("koru")),
            }
        }
        Ok(Self {
            config: xdg("XDG_CONFIG_HOME", &home, ".config")?,
            cache: xdg("XDG_CACHE_HOME", &home, ".cache")?,
            state: xdg("XDG_STATE_HOME", &home, ".local/state")?,
        })
    }
    /// Directory containing top-level commands and the lib subdirectory.
    pub fn commands(&self) -> PathBuf {
        self.config.join("commands")
    }
}
