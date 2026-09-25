//! Terminal approval behavior with deterministic input and output.
use koru::{
    error::ErrorCode,
    permissions::{Decision, Environment, PreparedAction},
    runtime::{ExecutionContext, Limits},
    terminal::decide,
};
use std::{
    io::Cursor,
    time::{Duration, Instant},
};

fn action() -> PreparedAction {
    let context =
        ExecutionContext::new("demo", [1; 32], Limits::default(), Instant::now()).unwrap();
    PreparedAction::process(
        &context,
        std::env::current_exe().unwrap(),
        vec!["a\x1b[31mb".into()],
        std::env::temp_dir(),
        Environment::Clean,
    )
    .unwrap()
}

#[test]
fn exact_yes_approves_and_preview_escapes_controls() {
    let mut input = Cursor::new(b"yes\n".to_vec());
    let mut output = Vec::new();
    let decision = decide(
        &action(),
        &mut input,
        &mut output,
        true,
        Instant::now() + Duration::from_secs(1),
    )
    .unwrap();
    assert!(matches!(decision, Decision::ApproveOnce));
    let preview = String::from_utf8(output).unwrap();
    assert!(preview.contains("a\\u{1b}[31mb"), "{preview}");
    assert!(!preview.contains('\x1b'));
}

#[test]
fn denial_and_redirected_input_never_authorize() {
    let mut input = Cursor::new(b"y\n".to_vec());
    let mut output = Vec::new();
    assert!(matches!(
        decide(
            &action(),
            &mut input,
            &mut output,
            true,
            Instant::now() + Duration::from_secs(1)
        )
        .unwrap(),
        Decision::Deny
    ));
    let mut input = Cursor::new(b"yes\n".to_vec());
    let mut output = Vec::new();
    assert_eq!(
        decide(
            &action(),
            &mut input,
            &mut output,
            false,
            Instant::now() + Duration::from_secs(1)
        )
        .unwrap_err()
        .code(),
        ErrorCode::PermissionDenied
    );
    assert!(output.is_empty());
}

#[test]
fn eof_is_denial() {
    let mut input = Cursor::new(Vec::<u8>::new());
    let mut output = Vec::new();
    assert!(matches!(
        decide(
            &action(),
            &mut input,
            &mut output,
            true,
            Instant::now() + Duration::from_secs(1)
        )
        .unwrap(),
        Decision::Deny
    ));
}
