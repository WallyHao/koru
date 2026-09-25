//! Read-only Git snapshot and plan validation for the plan-only commit command.
mod plan;
mod snapshot;

pub(crate) use plan::validate_plan;
pub(crate) use snapshot::capture;
