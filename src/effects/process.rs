//! Bounded process execution with one-use approval and process-group cleanup.
use crate::permissions::action::Operation;
use crate::{
    error::{ErrorCode, KoruError, Result},
    permissions::{ApprovedAction, Environment},
    runtime::{ExecutionContext, Resources},
};
use std::{
    fs::File,
    io::Read,
    process::{Command, Stdio},
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

const STREAM_CAP: usize = 64 * 1024;
const WAIT_TICK: Duration = Duration::from_millis(10);
const DRAIN_GRACE: Duration = Duration::from_millis(250);

/// Bounded result of an approved child process.
#[derive(Debug)]
pub struct ProcessResult {
    /// Normal exit code, if the process exited normally.
    pub exit_code: Option<i32>,
    /// Terminating signal on Unix, if available.
    pub signal: Option<i32>,
    /// Captured stdout up to the stream cap.
    pub stdout: Vec<u8>,
    /// Captured stderr up to the stream cap.
    pub stderr: Vec<u8>,
    /// Whether stdout exceeded the cap.
    pub stdout_truncated: bool,
    /// Whether stderr exceeded the cap.
    pub stderr_truncated: bool,
}

struct Stream {
    bytes: Vec<u8>,
    truncated: bool,
}

/// Consume an approved process or shell action exactly once.
pub fn execute_process(
    mut approved: ApprovedAction,
    context: &ExecutionContext,
) -> Result<ProcessResult> {
    let action = approved.begin(context, Instant::now())?;
    #[cfg(not(target_os = "linux"))]
    return Err(KoruError::new(
        ErrorCode::UnsupportedCapability,
        "handle-bound process execution requires Linux",
    ));
    #[cfg(target_os = "linux")]
    let (executable_handle, cwd_handle) = match &action.operation {
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
            let executable_handle = open_identical(executable, *executable_identity)?;
            let cwd_handle = open_identical(cwd, *cwd_identity)?;
            (executable_handle, cwd_handle)
        }
        _ => {
            return Err(KoruError::new(
                ErrorCode::Validation,
                "approved action is not a process",
            ));
        }
    };
    #[cfg(target_os = "linux")]
    let executable_path = descriptor_path(&executable_handle);
    #[cfg(target_os = "linux")]
    let cwd_path = descriptor_path(&cwd_handle);
    let mut command = match &action.operation {
        Operation::Process {
            arguments,
            path_env,
            environment,
            ..
        } => {
            let mut command = Command::new(&executable_path);
            command.args(arguments).current_dir(&cwd_path);
            command.env_clear().env("PATH", path_env).env("LANG", "C");
            if let Environment::Additions(additions) = environment {
                for (name, value) in additions {
                    command.env(name, value);
                }
            }
            command
        }
        Operation::Shell {
            script, path_env, ..
        } => {
            let mut command = Command::new(&executable_path);
            command.arg("-c").arg(script).current_dir(&cwd_path);
            command.env_clear().env("PATH", path_env).env("LANG", "C");
            command
        }
        _ => {
            return Err(KoruError::new(
                ErrorCode::Validation,
                "approved action is not a process",
            ));
        }
    };
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    #[cfg(not(unix))]
    return Err(KoruError::new(
        ErrorCode::UnsupportedCapability,
        "process-group cleanup is unavailable on this platform",
    ));
    let mut child = command
        .spawn()
        .map_err(|error| KoruError::io("cannot spawn approved process", error))?;
    let group = child.id();
    let (sender, receiver) = mpsc::channel();
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| KoruError::new(ErrorCode::Io, "child stdout unavailable"))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| KoruError::new(ErrorCode::Io, "child stderr unavailable"))?;
    let pipes: [Box<dyn Read + Send>; 2] = [Box::new(stdout), Box::new(stderr)];
    for (index, mut pipe) in pipes.into_iter().enumerate() {
        let sender = sender.clone();
        let context = context.clone();
        thread::spawn(move || {
            let mut bytes = Vec::new();
            let mut truncated = false;
            let mut chunk = [0_u8; 4096];
            loop {
                match pipe.read(&mut chunk) {
                    Ok(0) => break,
                    Ok(count) => {
                        if context
                            .reserve(
                                Resources {
                                    bytes: count as u64,
                                    ..Resources::ZERO
                                },
                                Instant::now(),
                            )
                            .is_err()
                        {
                            break;
                        }
                        let retained = (STREAM_CAP - bytes.len()).min(count);
                        bytes.extend_from_slice(&chunk[..retained]);
                        truncated |= retained < count;
                    }
                    Err(_) => break,
                }
            }
            let _ = sender.send((index, Stream { bytes, truncated }));
        });
    }
    drop(sender);
    let status = loop {
        if let Err(error) = context.ensure_active(Instant::now()) {
            terminate_group(group, &mut child);
            return Err(error);
        }
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => thread::sleep(WAIT_TICK),
            Err(error) => {
                terminate_group(group, &mut child);
                return Err(KoruError::io("cannot wait for approved process", error));
            }
        }
    };
    let mut streams: [Option<Stream>; 2] = [None, None];
    for _ in 0..2 {
        match receiver.recv_timeout(DRAIN_GRACE) {
            Ok((index, stream)) => streams[index] = Some(stream),
            Err(_) => {
                terminate_group(group, &mut child);
                return Err(KoruError::new(
                    ErrorCode::Timeout,
                    "process output did not close after exit",
                ));
            }
        }
    }
    let stdout = streams[0]
        .take()
        .ok_or_else(|| KoruError::new(ErrorCode::Io, "stdout reader stopped"))?;
    let stderr = streams[1]
        .take()
        .ok_or_else(|| KoruError::new(ErrorCode::Io, "stderr reader stopped"))?;
    #[cfg(unix)]
    let signal = {
        use std::os::unix::process::ExitStatusExt;
        status.signal()
    };
    #[cfg(not(unix))]
    let signal = None;
    Ok(ProcessResult {
        exit_code: status.code(),
        signal,
        stdout: stdout.bytes,
        stderr: stderr.bytes,
        stdout_truncated: stdout.truncated,
        stderr_truncated: stderr.truncated,
    })
}

fn terminate_group(group: u32, child: &mut std::process::Child) {
    #[cfg(unix)]
    if let Ok(raw) = i32::try_from(group)
        && let Some(pid) = rustix::process::Pid::from_raw(raw)
    {
        let _ = rustix::process::kill_process_group(pid, rustix::process::Signal::KILL);
    }
    let _ = child.kill();
    let _ = child.wait();
}

#[cfg(target_os = "linux")]
fn open_identical(
    path: &std::path::Path,
    identity: crate::permissions::action::PathIdentity,
) -> Result<File> {
    let handle = File::open(path).map_err(|error| KoruError::at_path(path.to_path_buf(), error))?;
    let current = crate::permissions::action::PathIdentity::of_metadata(
        &handle
            .metadata()
            .map_err(|error| KoruError::at_path(path.to_path_buf(), error))?,
    )?;
    if current != identity {
        return Err(KoruError::new(
            ErrorCode::StateConflict,
            "prepared process target changed",
        ));
    }
    Ok(handle)
}

#[cfg(target_os = "linux")]
fn descriptor_path(handle: &File) -> std::path::PathBuf {
    use std::os::fd::AsRawFd;
    std::path::PathBuf::from(format!("/proc/self/fd/{}", handle.as_raw_fd()))
}
