//! Terminal adapter: interactive input, approval, styling, and rendering.
//!
//! The adapter is the only component that prompts for operation approval and
//! the only one that decides how intended output and diagnostics are routed.
//! All user-facing text is English; escape codes are emitted only when the
//! target stream is an interactive terminal and `NO_COLOR` is unset.
mod input;
mod render;
mod style;

pub use input::{approve, decide, read_task};
pub use render::{exit_code, render_result};
pub use style::Style;
