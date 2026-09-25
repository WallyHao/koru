//! Koru-owned workflow boundaries; adapters must not widen their authority.

pub mod ai;
pub mod config;
pub mod credentials;
pub mod declaration;
pub mod error;
pub mod json;
pub mod lua;
pub mod paths;
pub mod permissions;
pub mod provider;
pub mod runtime;
pub mod schema;
pub mod source;
pub mod transport;

/// The command declaration API version planned for the first runtime.
pub const API_VERSION: u32 = 1;
