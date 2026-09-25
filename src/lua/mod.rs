//! Restricted Lua adapter: sandbox construction, module loading, declaration conversion.
mod bridge;
mod command;
mod declaration;
mod error;
mod json;
mod loader;
mod sandbox;

pub use command::LoadedCommand;
