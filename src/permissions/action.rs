//! Immutable effect descriptions; preview and executor receive the same value.
use crate::{
    error::{ErrorCode, KoruError, Result},
    runtime::ExecutionContext,
};
use std::{
    collections::BTreeMap,
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

static NEXT_ACTION_ID: AtomicU64 = AtomicU64::new(1);
const MAX_ACTION_BYTES: usize = 64 * 1024;
const MAX_ARGUMENTS: usize = 64;
const MAX_ENV_ADDITIONS: usize = 32;

/// Child environment policy; provider credentials are never inherited implicitly.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Environment {
    /// A minimal, adapter-owned environment.
    Clean,
    /// A minimal environment plus reviewed exact key/value additions.
    Additions(BTreeMap<String, String>),
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Operation {
    Process {
        executable: PathBuf,
        arguments: Vec<String>,
        cwd: PathBuf,
        environment: Environment,
    },
    Shell {
        executable: PathBuf,
        script: String,
        cwd: PathBuf,
    },
}
/// An immutable operation bound to one execution context and unique action ID.
#[derive(Debug)]
pub struct PreparedAction {
    pub(crate) id: u64,
    pub(crate) run_id: u64,
    pub(crate) command: String,
    pub(crate) source_digest: [u8; 32],
    pub(crate) operation: Operation,
}
impl PreparedAction {
    /// Prepare an exact direct-process invocation; does not execute it.
    pub fn process(
        context: &ExecutionContext,
        executable: PathBuf,
        arguments: Vec<String>,
        cwd: PathBuf,
        environment: Environment,
    ) -> Result<Self> {
        if arguments.len() > MAX_ARGUMENTS
            || arguments.iter().map(String::len).sum::<usize>() > MAX_ACTION_BYTES
        {
            return Err(KoruError::new(
                ErrorCode::BudgetExhausted,
                "process arguments exceed preparation limits",
            ));
        }
        if arguments.iter().any(|value| value.contains('\0')) {
            return Err(KoruError::new(
                ErrorCode::Validation,
                "process argument contains NUL",
            ));
        }
        if let Environment::Additions(values) = &environment {
            if values.len() > MAX_ENV_ADDITIONS
                || values
                    .iter()
                    .map(|(key, value)| key.len() + value.len())
                    .sum::<usize>()
                    > MAX_ACTION_BYTES
            {
                return Err(KoruError::new(
                    ErrorCode::BudgetExhausted,
                    "environment additions exceed preparation limits",
                ));
            }
            for (key, value) in values {
                if key.is_empty() || key.contains(['=', '\0']) || value.contains('\0') {
                    return Err(KoruError::new(
                        ErrorCode::Validation,
                        "invalid environment addition",
                    ));
                }
                if key.starts_with("KORU_") || key.ends_with("_API_KEY") {
                    return Err(KoruError::new(
                        ErrorCode::Validation,
                        "internal or provider credential environment names are reserved",
                    ));
                }
            }
        }
        let executable = canonical_file(executable)?;
        let cwd = canonical_dir(cwd)?;
        Ok(Self::new(
            context,
            Operation::Process {
                executable,
                arguments,
                cwd,
                environment,
            },
        ))
    }
    /// Prepare exact shell text for a fixed configured shell; never exempted.
    pub fn shell(
        context: &ExecutionContext,
        executable: PathBuf,
        script: String,
        cwd: PathBuf,
    ) -> Result<Self> {
        if script.len() > MAX_ACTION_BYTES {
            return Err(KoruError::new(
                ErrorCode::BudgetExhausted,
                "shell script exceeds preparation limit",
            ));
        }
        if script.contains('\0') {
            return Err(KoruError::new(
                ErrorCode::Validation,
                "shell script contains NUL",
            ));
        }
        let executable = canonical_file(executable)?;
        let cwd = canonical_dir(cwd)?;
        Ok(Self::new(
            context,
            Operation::Shell {
                executable,
                script,
                cwd,
            },
        ))
    }
    fn new(context: &ExecutionContext, operation: Operation) -> Self {
        Self {
            id: NEXT_ACTION_ID.fetch_add(1, Ordering::Relaxed),
            run_id: context.run_id(),
            command: context.command().to_owned(),
            source_digest: context.source_digest(),
            operation,
        }
    }
    /// Unique identifier within this CLI process.
    pub fn id(&self) -> u64 {
        self.id
    }
    /// A bounded, escaped textual preview of this exact operation.
    pub fn display_preview(&self) -> String {
        // Keep arbitrary model text from injecting terminal control sequences.
        fn escaped(value: &str) -> String {
            value.chars().flat_map(char::escape_default).collect()
        }
        match &self.operation {
            Operation::Process {
                executable,
                arguments,
                cwd,
                environment,
            } => {
                let args = arguments
                    .iter()
                    .map(|arg| escaped(arg))
                    .collect::<Vec<_>>()
                    .join(" | ");
                format!(
                    "direct process: {} [{}] in {} env={environment:?}",
                    escaped(&executable.to_string_lossy()),
                    args,
                    escaped(&cwd.to_string_lossy())
                )
            }
            Operation::Shell {
                executable,
                script,
                cwd,
            } => format!(
                "shell: {} in {}\nscript: {}",
                escaped(&executable.to_string_lossy()),
                escaped(&cwd.to_string_lossy()),
                escaped(script)
            ),
        }
    }
    pub(crate) fn matches(&self, context: &ExecutionContext) -> bool {
        self.run_id == context.run_id()
            && self.command == context.command()
            && self.source_digest == context.source_digest()
    }
}
fn canonical_file(path: PathBuf) -> Result<PathBuf> {
    let canonical = fs::canonicalize(&path).map_err(|error| KoruError::at_path(path, error))?;
    let metadata =
        fs::metadata(&canonical).map_err(|error| KoruError::at_path(canonical.clone(), error))?;
    if !metadata.is_file() {
        return Err(KoruError::new(
            ErrorCode::Validation,
            "executable must be a regular file",
        ));
    }
    Ok(canonical)
}
fn canonical_dir(path: PathBuf) -> Result<PathBuf> {
    let canonical = fs::canonicalize(&path).map_err(|error| KoruError::at_path(path, error))?;
    let metadata =
        fs::metadata(&canonical).map_err(|error| KoruError::at_path(canonical.clone(), error))?;
    if !metadata.is_dir() {
        return Err(KoruError::new(
            ErrorCode::Validation,
            "working directory must be a directory",
        ));
    }
    Ok(canonical)
}
