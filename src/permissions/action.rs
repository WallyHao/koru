//! Immutable effect descriptions; preview and executor receive the same value.
use crate::{
    error::{ErrorCode, KoruError, Result},
    runtime::ExecutionContext,
};
use sha2::{Digest, Sha256};
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
        executable_identity: PathIdentity,
        arguments: Vec<String>,
        cwd: PathBuf,
        cwd_identity: PathIdentity,
        environment: Environment,
    },
    Shell {
        executable: PathBuf,
        executable_identity: PathIdentity,
        script: String,
        cwd: PathBuf,
        cwd_identity: PathIdentity,
    },
    FileRead {
        path: PathBuf,
        identity: PathIdentity,
    },
    FileWrite {
        path: PathBuf,
        parent: PathBuf,
        parent_identity: PathIdentity,
        expected: Option<PathIdentity>,
        bytes: Vec<u8>,
    },
    DirectoryList {
        path: PathBuf,
        identity: PathIdentity,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct PathIdentity {
    #[cfg(unix)]
    device: u64,
    #[cfg(unix)]
    inode: u64,
}
impl PathIdentity {
    fn of(path: &PathBuf) -> Result<Self> {
        let metadata =
            fs::symlink_metadata(path).map_err(|error| KoruError::at_path(path.clone(), error))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            Ok(Self {
                device: metadata.dev(),
                inode: metadata.ino(),
            })
        }
        #[cfg(not(unix))]
        {
            let _ = metadata;
            Err(KoruError::new(
                ErrorCode::UnsupportedCapability,
                "path identity checks are unavailable on this platform",
            ))
        }
    }
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
    /// Prepare a bounded data-file read.
    pub fn file_read(context: &ExecutionContext, path: PathBuf) -> Result<Self> {
        let path = canonical_file(path)?;
        let identity = PathIdentity::of(&path)?;
        Ok(Self::new(context, Operation::FileRead { path, identity }))
    }

    /// Prepare a data-file write of exact reviewed bytes.
    pub fn file_write(context: &ExecutionContext, path: PathBuf, bytes: Vec<u8>) -> Result<Self> {
        if bytes.len() > MAX_ACTION_BYTES {
            return Err(KoruError::new(
                ErrorCode::BudgetExhausted,
                "file write exceeds preparation limit",
            ));
        }
        let name = path
            .file_name()
            .ok_or_else(|| KoruError::new(ErrorCode::Validation, "file write needs a filename"))?;
        let parent = path.parent().ok_or_else(|| {
            KoruError::new(ErrorCode::Validation, "file write needs a parent directory")
        })?;
        let parent = canonical_dir(parent.to_path_buf())?;
        let parent_identity = PathIdentity::of(&parent)?;
        let path = parent.join(name);
        let expected = match fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(KoruError::new(
                    ErrorCode::Validation,
                    "file write target cannot be a symlink",
                ));
            }
            Ok(metadata) if !metadata.is_file() => {
                return Err(KoruError::new(
                    ErrorCode::Validation,
                    "file write target must be a regular file",
                ));
            }
            Ok(_) => Some(PathIdentity::of(&path)?),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(KoruError::at_path(path, error)),
        };
        Ok(Self::new(
            context,
            Operation::FileWrite {
                path,
                parent,
                parent_identity,
                expected,
                bytes,
            },
        ))
    }

    /// Prepare a bounded directory listing.
    pub fn directory_list(context: &ExecutionContext, path: PathBuf) -> Result<Self> {
        let path = canonical_dir(path)?;
        let identity = PathIdentity::of(&path)?;
        Ok(Self::new(
            context,
            Operation::DirectoryList { path, identity },
        ))
    }
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
        let executable_identity = PathIdentity::of(&executable)?;
        let cwd_identity = PathIdentity::of(&cwd)?;
        Ok(Self::new(
            context,
            Operation::Process {
                executable,
                executable_identity,
                arguments,
                cwd,
                cwd_identity,
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
        let executable_identity = PathIdentity::of(&executable)?;
        let cwd_identity = PathIdentity::of(&cwd)?;
        Ok(Self::new(
            context,
            Operation::Shell {
                executable,
                executable_identity,
                script,
                cwd,
                cwd_identity,
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
    /// Command name bound to this prepared effect.
    pub fn command(&self) -> &str {
        &self.command
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
                ..
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
                ..
            } => format!(
                "shell: {} in {}\nscript: {}",
                escaped(&executable.to_string_lossy()),
                escaped(&cwd.to_string_lossy()),
                escaped(script)
            ),
            Operation::FileRead { path, .. } => {
                format!("read file: {}", escaped(&path.to_string_lossy()))
            }
            Operation::DirectoryList { path, .. } => {
                format!("list directory: {}", escaped(&path.to_string_lossy()))
            }
            Operation::FileWrite { path, bytes, .. } => {
                let digest = Sha256::digest(bytes);
                format!(
                    "write file: {} ({} bytes, sha256 {digest:x})",
                    escaped(&path.to_string_lossy()),
                    bytes.len()
                )
            }
        }
    }
    pub(crate) fn revalidate(&self) -> Result<()> {
        let unchanged = match &self.operation {
            Operation::Process {
                executable,
                executable_identity,
                cwd,
                cwd_identity,
                ..
            }
            | Operation::Shell {
                executable,
                executable_identity,
                cwd,
                cwd_identity,
                ..
            } => {
                PathIdentity::of(executable).is_ok_and(|current| current == *executable_identity)
                    && PathIdentity::of(cwd).is_ok_and(|current| current == *cwd_identity)
            }
            Operation::FileRead { path, identity }
            | Operation::DirectoryList { path, identity } => {
                PathIdentity::of(path).is_ok_and(|current| current == *identity)
            }
            Operation::FileWrite {
                path,
                parent,
                parent_identity,
                expected,
                ..
            } => {
                PathIdentity::of(parent).is_ok_and(|current| current == *parent_identity)
                    && match expected {
                        Some(expected) => {
                            PathIdentity::of(path).is_ok_and(|current| current == *expected)
                        }
                        None => {
                            matches!(fs::symlink_metadata(path), Err(error) if error.kind() == std::io::ErrorKind::NotFound)
                        }
                    }
            }
        };
        if unchanged {
            Ok(())
        } else {
            Err(KoruError::new(
                ErrorCode::StateConflict,
                "prepared action target changed",
            ))
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
