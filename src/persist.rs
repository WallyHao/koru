//! Atomic file replacement and scoped writer locks.
//!
//! Persistent user files are written by creating a same-directory temporary file,
//! flushing it, and renaming it into place. Read-modify-write is serialized by an
//! exclusively created lock file with a bounded wait. A lock older than
//! [`STALE_LOCK_AFTER`] is treated as abandoned and replaced; anything younger
//! fails with actionable guidance instead of being removed.
use crate::error::{ErrorCode, KoruError, Result};
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    thread,
    time::{Duration, Instant, SystemTime},
};

/// How long a writer waits for an existing lock before failing.
pub const LOCK_WAIT: Duration = Duration::from_millis(1000);
/// How long a lock may exist before it is considered abandoned.
pub const STALE_LOCK_AFTER: Duration = Duration::from_secs(60);
const LOCK_RETRY: Duration = Duration::from_millis(5);

/// An exclusively created lock file removed when the guard drops.
#[derive(Debug)]
pub struct FileLock {
    path: PathBuf,
}
impl FileLock {
    /// Acquire the lock for `target`, breaking an abandoned lock if necessary.
    pub fn acquire(target: &Path) -> Result<Self> {
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
                    if is_stale(&path) {
                        match fs::remove_file(&path) {
                            Ok(()) => continue,
                            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                            Err(error) => {
                                return Err(KoruError::at_path(path.clone(), error));
                            }
                        }
                    }
                    if Instant::now() >= deadline {
                        return Err(KoruError::new(
                            ErrorCode::StateConflict,
                            format!(
                                "lock {} is held; remove it if no writer is running",
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
impl Drop for FileLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

fn is_stale(path: &Path) -> bool {
    let Ok(metadata) = fs::metadata(path) else {
        return true;
    };
    let Ok(modified) = metadata.modified() else {
        return false;
    };
    SystemTime::now()
        .duration_since(modified)
        .map(|age| age >= STALE_LOCK_AFTER)
        .unwrap_or(false)
}

/// Replace `path` atomically with `bytes`, creating parent directories.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent).map_err(|error| KoruError::at_path(parent.to_path_buf(), error))?;
    let temp = temp_path(parent);
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

fn temp_path(parent: &Path) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|value| value.as_nanos())
        .unwrap_or(0);
    parent.join(format!(".koru-{}.{nanos}.tmp", std::process::id()))
}

fn lock_path(target: &Path) -> PathBuf {
    let mut name = target.as_os_str().to_owned();
    name.push(".lock");
    PathBuf::from(name)
}
