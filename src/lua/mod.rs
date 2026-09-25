//! Restricted Lua adapter: sandbox construction, module loading, declaration conversion.
mod command;
mod declaration;
mod error;
mod loader;
mod sandbox;

pub use command::LoadedCommand;
