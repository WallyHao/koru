//! Policy boundaries: a script's request is never a host grant.
use koru::{
    error::ErrorCode,
    permissions::{Broker, Decision, Environment, Policy, PreparedAction, RequestedCapabilities},
    runtime::{ExecutionContext, Limits},
};
use std::{collections::BTreeMap, path::PathBuf, time::Instant};

fn context(digest: [u8; 32]) -> ExecutionContext {
    ExecutionContext::new("demo", digest, Limits::default(), Instant::now()).unwrap()
}
fn process(context: &ExecutionContext, args: &[&str], env: Environment) -> PreparedAction {
    PreparedAction::process(
        context,
        std::env::current_exe().unwrap(),
        args.iter().map(|item| item.to_string()).collect(),
        PathBuf::from("/tmp"),
        env,
    )
    .unwrap()
}
#[test]
fn script_requests_do_not_grant_direct_process_access() {
    let context = context([1; 32]);
    let requested = RequestedCapabilities {
        direct_processes: true,
    };
    let action = process(&context, &["ok"], Environment::Clean);
    let policy = Policy::new(1).unwrap();
    assert!(requested.direct_processes);
    assert_eq!(
        Broker::authorize(action, &context, &policy, Decision::Deny, Instant::now())
            .unwrap_err()
            .code(),
        ErrorCode::PermissionDenied
    );
}
#[test]
fn exact_grants_bind_source_policy_arguments_and_environment() {
    let original = context([1; 32]);
    let mut policy = Policy::new(1).unwrap();
    let action = process(&original, &["hello"], Environment::Clean);
    policy.grant_exact_process(&action, &original).unwrap();
    assert!(
        Broker::authorize(
            process(&original, &["hello"], Environment::Clean),
            &original,
            &policy,
            Decision::Deny,
            Instant::now()
        )
        .is_ok()
    );
    assert!(
        Broker::authorize(
            process(&original, &["other"], Environment::Clean),
            &original,
            &policy,
            Decision::Deny,
            Instant::now()
        )
        .is_err()
    );
    let changed = context([2; 32]);
    assert!(
        Broker::authorize(
            process(&changed, &["hello"], Environment::Clean),
            &changed,
            &policy,
            Decision::Deny,
            Instant::now()
        )
        .is_err()
    );
    let newer_policy = Policy::new(2).unwrap();
    assert!(
        Broker::authorize(
            process(&original, &["hello"], Environment::Clean),
            &original,
            &newer_policy,
            Decision::Deny,
            Instant::now()
        )
        .is_err()
    );
    let mut additions = BTreeMap::new();
    additions.insert("LANG".to_owned(), "C".to_owned());
    assert!(
        Broker::authorize(
            process(&original, &["hello"], Environment::Additions(additions)),
            &original,
            &policy,
            Decision::Deny,
            Instant::now()
        )
        .is_err()
    );
}
#[test]
fn shell_script_never_uses_direct_process_exemption() {
    let context = context([1; 32]);
    let mut policy = Policy::new(1).unwrap();
    let direct = process(&context, &["hello"], Environment::Clean);
    policy.grant_exact_process(&direct, &context).unwrap();
    let shell = PreparedAction::shell(
        &context,
        std::env::current_exe().unwrap(),
        "echo hello".into(),
        PathBuf::from("/tmp"),
    )
    .unwrap();
    assert_eq!(
        policy
            .grant_exact_process(&shell, &context)
            .unwrap_err()
            .code(),
        ErrorCode::Validation
    );
    assert_eq!(
        Broker::authorize(shell, &context, &policy, Decision::Deny, Instant::now())
            .unwrap_err()
            .code(),
        ErrorCode::PermissionDenied
    );
}
#[test]
fn authorization_is_bound_to_context_and_single_use() {
    let first = context([1; 32]);
    let second = context([1; 32]);
    let policy = Policy::new(1).unwrap();
    let action = process(&first, &["ok"], Environment::Clean);
    assert_eq!(
        Broker::authorize(
            action,
            &second,
            &policy,
            Decision::ApproveOnce,
            Instant::now()
        )
        .unwrap_err()
        .code(),
        ErrorCode::StateConflict
    );
    let mut approved = Broker::authorize(
        process(&first, &["ok"], Environment::Clean),
        &first,
        &policy,
        Decision::ApproveOnce,
        Instant::now(),
    )
    .unwrap();
    assert!(approved.action().display_preview().contains("ok"));
    approved.begin(&first, Instant::now()).unwrap();
    assert_eq!(
        approved.begin(&first, Instant::now()).unwrap_err().code(),
        ErrorCode::StateConflict
    );
    assert_eq!(first.usage().unwrap().effects, 1);
}
#[test]
fn cancellation_after_approval_stops_execution() {
    let context = context([1; 32]);
    let policy = Policy::new(1).unwrap();
    let mut approved = Broker::authorize(
        process(&context, &["ok"], Environment::Clean),
        &context,
        &policy,
        Decision::ApproveOnce,
        Instant::now(),
    )
    .unwrap();
    context.cancel().unwrap();
    assert_eq!(
        approved.begin(&context, Instant::now()).unwrap_err().code(),
        ErrorCode::Cancelled
    );
    assert_eq!(context.usage().unwrap().effects, 0);
}

#[test]
fn preparation_rejects_unbounded_or_controlled_inputs() {
    let context = context([1; 32]);
    let long = "x".repeat(65 * 1024);
    assert_eq!(
        PreparedAction::shell(
            &context,
            std::env::current_exe().unwrap(),
            long,
            PathBuf::from("/tmp")
        )
        .unwrap_err()
        .code(),
        ErrorCode::BudgetExhausted
    );
    assert_eq!(
        PreparedAction::process(
            &context,
            std::env::current_exe().unwrap(),
            vec!["x".repeat(65 * 1024)],
            PathBuf::from("/tmp"),
            Environment::Clean
        )
        .unwrap_err()
        .code(),
        ErrorCode::BudgetExhausted
    );
    let mut env = BTreeMap::new();
    env.insert("DEEPSEEK_API_KEY".to_owned(), "secret".to_owned());
    assert_eq!(
        PreparedAction::process(
            &context,
            std::env::current_exe().unwrap(),
            vec![],
            PathBuf::from("/tmp"),
            Environment::Additions(env)
        )
        .unwrap_err()
        .code(),
        ErrorCode::Validation
    );
}
