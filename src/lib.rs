//! Koru-owned workflow boundaries; adapters must not widen their authority.

pub mod declaration;
pub mod error;
pub mod lua;
pub mod paths;
pub mod permissions;
pub mod runtime;
pub mod source;

/// The command declaration API version planned for the first runtime.
pub const API_VERSION: u32 = 1;
