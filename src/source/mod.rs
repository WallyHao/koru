//! Bounded, immutable command source capture without Lua evaluation.
mod bundle;
mod discovery;
mod names;
mod reader;

pub use bundle::{CapturedSource, SourceBundle, SourceLimits};
pub use discovery::discover;
