//! Provider metadata and capability boundaries.
pub mod catalog;
mod chat;
pub mod deepseek;
mod http;
pub mod protocol;

pub use catalog::{Catalog, CatalogEntry, CatalogSource, ServiceId};
pub use deepseek::DeepSeekAdapter;
