//! Approved process execution keeps arguments, output, and credentials bounded.
use koru::{
    effects::execute_process,
    permissions::{Broker, Decision, Environment, Policy, PreparedAction},
    runtime::{ExecutionContext, Limits},
};
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

fn context() -> ExecutionContext {
    ExecutionContext::new("demo", [5; 32], Limits::default(), Instant::now()).unwrap()
}

#[test]
fn approved_shell_uses_exact_script_and_clean_environment() {
    let context = context();
    let script = "printf 'ok'; test -z \"$DEEPSEEK_API_KEY\"".to_owned();
    let action = PreparedAction::shell(
        &context,
        PathBuf::from("/bin/sh"),
        script,
        std::env::temp_dir(),
    )
    .unwrap();
    let policy = Policy::new(1).unwrap();
    let approved = Broker::authorize(
        action,
        &context,
        &policy,
        Decision::ApproveOnce,
        Instant::now(),
    )
    .unwrap();
    let result = execute_process(approved, &context).unwrap();
    assert_eq!(result.exit_code, Some(0));
    assert_eq!(result.stdout, b"ok");
    assert!(!result.stdout_truncated);
}

#[test]
fn nonzero_exit_is_a_result_and_streams_are_capped() {
    let context = context();
    let action = PreparedAction::shell(
        &context,
        PathBuf::from("/bin/sh"),
        "printf '%070000d' 0; exit 7".into(),
        std::env::temp_dir(),
    )
    .unwrap();
    let policy = Policy::new(1).unwrap();
    let approved = Broker::authorize(
        action,
        &context,
        &policy,
        Decision::ApproveOnce,
        Instant::now(),
    )
    .unwrap();
    let result = execute_process(approved, &context).unwrap();
    assert_eq!(result.exit_code, Some(7));
    assert_eq!(
        result.stdout.len(),
        65536,
        "stderr: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(result.stdout_truncated);
}

#[test]
fn whole_command_deadline_stops_a_running_shell() {
    let limits = Limits {
        wall_time: Duration::from_millis(50),
        ..Limits::default()
    };
    let context = ExecutionContext::new("demo", [6; 32], limits, Instant::now()).unwrap();
    let action = PreparedAction::shell(
        &context,
        PathBuf::from("/bin/sh"),
        "while :; do :; done".into(),
        std::env::temp_dir(),
    )
    .unwrap();
    let policy = Policy::new(1).unwrap();
    let approved = Broker::authorize(
        action,
        &context,
        &policy,
        Decision::ApproveOnce,
        Instant::now(),
    )
    .unwrap();
    let started = Instant::now();
    let error = execute_process(approved, &context).unwrap_err();
    assert_eq!(error.code(), koru::error::ErrorCode::Timeout);
    assert!(started.elapsed() < Duration::from_secs(2));
}

#[test]
fn direct_process_preserves_literal_arguments() {
    let context = context();
    let action = PreparedAction::process(
        &context,
        PathBuf::from("/bin/sh"),
        vec![
            "-c".into(),
            "printf '%s' \"$1\"".into(),
            "--".into(),
            "a*b".into(),
        ],
        std::env::temp_dir(),
        Environment::Clean,
    )
    .unwrap();
    let policy = Policy::new(1).unwrap();
    let approved = Broker::authorize(
        action,
        &context,
        &policy,
        Decision::ApproveOnce,
        Instant::now(),
    )
    .unwrap();
    assert_eq!(execute_process(approved, &context).unwrap().stdout, b"a*b");
}

#[test]
fn changed_executable_cannot_use_an_earlier_approval() {
    let root = tempfile::tempdir().unwrap();
    let executable = root.path().join("program");
    std::fs::copy("/bin/sh", &executable).unwrap();
    let context = context();
    let action = PreparedAction::process(
        &context,
        executable.clone(),
        vec![],
        root.path().to_path_buf(),
        Environment::Clean,
    )
    .unwrap();
    let policy = Policy::new(1).unwrap();
    let approved = Broker::authorize(
        action,
        &context,
        &policy,
        Decision::ApproveOnce,
        Instant::now(),
    )
    .unwrap();
    std::fs::remove_file(&executable).unwrap();
    std::fs::copy("/bin/sh", &executable).unwrap();
    assert_eq!(
        execute_process(approved, &context).unwrap_err().code(),
        koru::error::ErrorCode::StateConflict
    );
}
