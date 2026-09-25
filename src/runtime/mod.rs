//! Shared, Rust-owned command execution state.
mod budget;
pub use budget::{ExecutionContext, Limits, Resources, RunState};
