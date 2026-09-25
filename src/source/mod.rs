//! Bounded, immutable command source capture without Lua evaluation.
mod bundle;
mod discovery;
pub(crate) mod names;
mod reader;

pub use bundle::{CapturedSource, SourceBundle, SourceLimits};
pub use discovery::discover;
