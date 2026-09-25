//! Credential resolution and redaction contracts.
use koru::{
    credentials::{Credentials, DEEPSEEK_API_KEY, MapEnvironment, OPENCODE_API_KEY, redact},
    error::ErrorCode,
};

#[test]
fn reads_present_variables_only() {
    let env = MapEnvironment::new([(DEEPSEEK_API_KEY, "ds-secret")]);
    let credentials = Credentials::from_environment(&env);
    assert!(credentials.is_set(DEEPSEEK_API_KEY));
    assert!(!credentials.is_set(OPENCODE_API_KEY));
    assert_eq!(credentials.require(DEEPSEEK_API_KEY).unwrap(), "ds-secret");
}

#[test]
fn missing_variables_fail_with_an_actionable_error() {
    let credentials = Credentials::from_environment(&MapEnvironment::default());
    let error = credentials.require(DEEPSEEK_API_KEY).unwrap_err();
    assert_eq!(error.code(), ErrorCode::Validation);
    assert!(error.message().contains(DEEPSEEK_API_KEY));
}

#[test]
fn empty_values_count_as_missing() {
    let env = MapEnvironment::new([(OPENCODE_API_KEY, "")]);
    let credentials = Credentials::from_environment(&env);
    assert!(!credentials.is_set(OPENCODE_API_KEY));
    assert_eq!(
        credentials.require(OPENCODE_API_KEY).unwrap_err().code(),
        ErrorCode::Validation
    );
}

#[test]
fn redaction_removes_known_secrets() {
    let env = MapEnvironment::new([
        (DEEPSEEK_API_KEY, "ds-secret"),
        (OPENCODE_API_KEY, "oc-secret"),
    ]);
    let credentials = Credentials::from_environment(&env);
    let text = "provider failed with key ds-secret and oc-secret attached";
    let cleaned = credentials.redact(text);
    assert!(!cleaned.contains("ds-secret"));
    assert!(!cleaned.contains("oc-secret"));
    assert!(cleaned.contains("[redacted]"));
    assert!(cleaned.contains("provider failed"));
}

#[test]
fn redaction_ignores_empty_secrets() {
    assert_eq!(redact([""], "nothing to do"), "nothing to do");
}

#[test]
fn debug_output_does_not_leak_values() {
    let env = MapEnvironment::new([(DEEPSEEK_API_KEY, "ds-secret")]);
    let credentials = Credentials::from_environment(&env);
    let rendered = format!("{credentials:?}");
    assert!(!rendered.contains("ds-secret"));
    assert!(rendered.contains(DEEPSEEK_API_KEY));
}
