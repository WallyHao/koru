//! Shared-budget and cancellation tests across cloned command contexts.
use koru::{
    error::ErrorCode,
    runtime::{ExecutionContext, Limits, Resources, RunState},
};
use std::time::{Duration, Instant};

fn context(limits: Limits, now: Instant) -> ExecutionContext {
    ExecutionContext::new("demo", [7; 32], limits, now).unwrap()
}
#[test]
fn reservations_are_atomic_across_all_resources_and_clones() {
    let limits = Limits {
        model_requests: 2,
        tool_calls: 1,
        effects: 2,
        bytes: 100,
        ..Limits::default()
    };
    let now = Instant::now();
    let context = context(limits, now);
    let other = context.clone();
    context
        .reserve(
            Resources {
                model_requests: 1,
                effects: 1,
                bytes: 20,
                ..Resources::ZERO
            },
            now,
        )
        .unwrap();
    let failed = other.reserve(
        Resources {
            model_requests: 1,
            effects: 2,
            ..Resources::ZERO
        },
        now,
    );
    assert_eq!(failed.unwrap_err().code(), ErrorCode::BudgetExhausted);
    assert_eq!(context.usage().unwrap().effects, 1);
    assert_eq!(context.state().unwrap(), RunState::Exhausted);
    assert_eq!(
        context.reserve(Resources::ZERO, now).unwrap_err().code(),
        ErrorCode::BudgetExhausted
    );
}
#[test]
fn cancellation_is_terminal_even_after_existing_authorization() {
    let now = Instant::now();
    let context = context(Limits::default(), now);
    context.cancel().unwrap();
    assert_eq!(context.state().unwrap(), RunState::Cancelled);
    assert_eq!(
        context
            .reserve(
                Resources {
                    effects: 1,
                    ..Resources::ZERO
                },
                now
            )
            .unwrap_err()
            .code(),
        ErrorCode::Cancelled
    );
}
#[test]
fn deadline_is_terminal_and_checked_before_reservation() {
    let start = Instant::now();
    let limits = Limits {
        wall_time: Duration::from_millis(2),
        ..Limits::default()
    };
    let context = context(limits, start);
    let future = start + Duration::from_millis(3);
    assert_eq!(
        context.reserve(Resources::ZERO, future).unwrap_err().code(),
        ErrorCode::Timeout
    );
    assert_eq!(context.state().unwrap(), RunState::TimedOut);
    assert_eq!(context.usage().unwrap(), Resources::ZERO);
}
#[test]
fn hard_ceilings_and_overflow_are_rejected() {
    let limits = Limits {
        effects: u64::MAX,
        ..Limits::default()
    };
    assert_eq!(
        ExecutionContext::new("demo", [0; 32], limits, Instant::now())
            .unwrap_err()
            .code(),
        ErrorCode::Validation
    );
}
#[test]
fn identity_is_shared_and_bound_to_command_and_source() {
    let now = Instant::now();
    let first = context(Limits::default(), now);
    assert_eq!(first.command(), "demo");
    assert_eq!(first.source_digest(), [7; 32]);
    assert_eq!(first.run_id(), first.clone().run_id());
    assert_ne!(first.run_id(), context(Limits::default(), now).run_id());
}
