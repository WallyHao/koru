//! Versioned user configuration: strict flat TOML subset, atomic writes, scoped lock.
//!
//! v1 `config.toml` is deliberately small: `schema_version = 1` plus optional
//! `provider`, `model`, and `variant` string keys. Only a flat subset of TOML is
//! accepted so the parser stays bounded and every unsupported construct fails with
//! a line-qualified error instead of being silently ignored. Read-modify-write
//! runs under a lock file and replaces the file atomically, so concurrent
//! selection cannot lose an update. Crash-safe stale-lock recovery belongs to the
//! persistence gate and is not promised here.
use crate::{
    error::{ErrorCode, KoruError, Result},
    paths::UserPaths,
};
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    thread,
    time::{Duration, Instant, SystemTime},
};

/// The only configuration schema version this build understands.
pub const CONFIG_SCHEMA_VERSION: u32 = 1;
/// Longest accepted provider, model, or variant string.
pub const MAX_VALUE_BYTES: usize = 256;
/// How long a writer waits for the configuration lock before failing.
const LOCK_WAIT: Duration = Duration::from_millis(1000);
const LOCK_RETRY: Duration = Duration::from_millis(5);

/// The selected provider/model/variant. Missing values mean "not selected".
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Config {
    /// Service identity such as `deepseek` or `opencode-go`.
    pub provider: Option<String>,
    /// Model identity within the service.
    pub model: Option<String>,
    /// Optional effort variant for the selected model.
    pub variant: Option<String>,
}
impl Config {
    /// Resolve the configuration path under the user's config directory.
    pub fn path(paths: &UserPaths) -> PathBuf {
        paths.config.join("config.toml")
    }

    /// Load the configuration, returning the default when the file is absent.
    pub fn load(path: &Path) -> Result<Self> {
        let text = match fs::read_to_string(path) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self::default());
            }
            Err(error) => return Err(KoruError::at_path(path.to_path_buf(), error)),
        };
        Self::parse(&text)
    }

    /// Parse the strict flat subset; every error names its line.
    pub fn parse(text: &str) -> Result<Self> {
        let mut config = Self::default();
        let mut schema_version = None;
        let mut provider_seen = false;
        let mut model_seen = false;
        let mut variant_seen = false;
        for (index, raw) in text.lines().enumerate() {
            let line = index + 1;
            let trimmed = raw.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }
            let Some((key, value)) = trimmed.split_once('=') else {
                return Err(parse_error(line, "expected `key = value`"));
            };
            let key = key.trim();
            let value = value.trim();
            match key {
                "schema_version" => {
                    if schema_version.is_some() {
                        return Err(parse_error(line, "duplicate `schema_version`"));
                    }
                    let version: u32 = value
                        .parse()
                        .map_err(|_| parse_error(line, "`schema_version` must be an integer"))?;
                    schema_version = Some(version);
                }
                "provider" => {
                    if provider_seen {
                        return Err(parse_error(line, "duplicate `provider`"));
                    }
                    provider_seen = true;
                    config.provider = Some(read_string(line, value)?);
                }
                "model" => {
                    if model_seen {
                        return Err(parse_error(line, "duplicate `model`"));
                    }
                    model_seen = true;
                    config.model = Some(read_string(line, value)?);
                }
                "variant" => {
                    if variant_seen {
                        return Err(parse_error(line, "duplicate `variant`"));
                    }
                    variant_seen = true;
                    config.variant = Some(read_string(line, value)?);
                }
                other => return Err(parse_error(line, format!("unknown key {other:?}"))),
            }
        }
        let version = schema_version.ok_or_else(|| parse_error(0, "missing `schema_version`"))?;
        if version != CONFIG_SCHEMA_VERSION {
            return Err(KoruError::new(
                ErrorCode::Validation,
                format!(
                    "unsupported config schema version {version}; this build supports {CONFIG_SCHEMA_VERSION}"
                ),
            ));
        }
        config.check_semantics()?;
        Ok(config)
    }

    /// Render the canonical file contents.
    pub fn render(&self) -> Result<String> {
        let mut out = format!("schema_version = {CONFIG_SCHEMA_VERSION}\n");
        for (key, value) in [
            ("provider", &self.provider),
            ("model", &self.model),
            ("variant", &self.variant),
        ] {
            if let Some(value) = value {
                out.push_str(&format!("{key} = \"{}\"\n", escape(value)?));
            }
        }
        Ok(out)
    }

    /// Read-modify-write under the scoped lock, replacing the file atomically.
    pub fn update(path: &Path, change: impl FnOnce(&mut Config)) -> Result<Self> {
        let _lock = Lock::acquire(path)?;
        let mut config = Self::load(path)?;
        change(&mut config);
        config.check_semantics()?;
        let rendered = config.render()?;
        write_atomic(path, rendered.as_bytes())?;
        Ok(config)
    }

    fn check_semantics(&self) -> Result<()> {
        if self.variant.is_some() && (self.provider.is_none() || self.model.is_none()) {
            return Err(KoruError::new(
                ErrorCode::Validation,
                "a variant requires a selected provider and model",
            ));
        }
        for value in [&self.provider, &self.model, &self.variant]
            .into_iter()
            .flatten()
        {
            validate_value(value)?;
        }
        Ok(())
    }
}

fn validate_value(value: &str) -> Result<()> {
    if value.is_empty() {
        return Err(KoruError::new(
            ErrorCode::Validation,
            "configuration values must not be empty",
        ));
    }
    if value.len() > MAX_VALUE_BYTES {
        return Err(KoruError::new(
            ErrorCode::Validation,
            format!("configuration value exceeds {MAX_VALUE_BYTES} bytes"),
        ));
    }
    if value.chars().any(char::is_control) {
        return Err(KoruError::new(
            ErrorCode::Validation,
            "configuration value contains control characters",
        ));
    }
    Ok(())
}

fn read_string(line: usize, value: &str) -> Result<String> {
    let Some(inner) = value
        .strip_prefix('"')
        .and_then(|rest| rest.strip_suffix('"'))
    else {
        return Err(parse_error(line, "value must be a double-quoted string"));
    };
    let mut out = String::with_capacity(inner.len());
    let mut chars = inner.chars();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            out.push(ch);
            continue;
        }
        match chars.next() {
            Some('"') => out.push('"'),
            Some('\\') => out.push('\\'),
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some(other) => {
                return Err(parse_error(line, format!("unsupported escape \\{other}")));
            }
            None => return Err(parse_error(line, "unterminated escape")),
        }
    }
    Ok(out)
}

fn escape(value: &str) -> Result<String> {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            ch if ch.is_control() => {
                return Err(KoruError::new(
                    ErrorCode::Validation,
                    "configuration value contains control characters",
                ));
            }
            ch => out.push(ch),
        }
    }
    Ok(out)
}

fn parse_error(line: usize, detail: impl Into<String>) -> KoruError {
    let detail = detail.into();
    if line == 0 {
        KoruError::new(ErrorCode::Validation, format!("invalid config: {detail}"))
    } else {
        KoruError::new(
            ErrorCode::Validation,
            format!("invalid config line {line}: {detail}"),
        )
    }
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent).map_err(|error| KoruError::at_path(parent.to_path_buf(), error))?;
    let nanos = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|value| value.as_nanos())
        .unwrap_or(0);
    let temp = parent.join(format!(".config.{}.{nanos}.tmp", std::process::id()));
    let result = (|| -> Result<()> {
        let mut file =
            fs::File::create(&temp).map_err(|error| KoruError::at_path(temp.clone(), error))?;
        file.write_all(bytes)
            .map_err(|error| KoruError::at_path(temp.clone(), error))?;
        file.sync_all()
            .map_err(|error| KoruError::at_path(temp.clone(), error))?;
        fs::rename(&temp, path).map_err(|error| KoruError::at_path(path.to_path_buf(), error))
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}

/// An exclusively created lock file removed when the guard drops.
struct Lock {
    path: PathBuf,
}
impl Lock {
    fn acquire(target: &Path) -> Result<Self> {
        let path = lock_path(target);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| KoruError::at_path(parent.to_path_buf(), error))?;
        }
        let deadline = Instant::now() + LOCK_WAIT;
        loop {
            match OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(mut file) => {
                    let _ = writeln!(file, "{}", std::process::id());
                    return Ok(Self { path });
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    if Instant::now() >= deadline {
                        return Err(KoruError::new(
                            ErrorCode::StateConflict,
                            format!(
                                "configuration lock {} is held; remove it if no writer is running",
                                path.display()
                            ),
                        ));
                    }
                    thread::sleep(LOCK_RETRY);
                }
                Err(error) => return Err(KoruError::at_path(path.clone(), error)),
            }
        }
    }
}
impl Drop for Lock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

fn lock_path(target: &Path) -> PathBuf {
    let mut name = target.as_os_str().to_owned();
    name.push(".lock");
    PathBuf::from(name)
}
