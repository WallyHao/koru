//! Provider metadata and capability boundaries.
pub mod cache;
pub mod catalog;
mod chat;
pub mod deepseek;
mod http;
pub mod opencode;
pub mod protocol;

pub use cache::CatalogCache;
pub use catalog::{Catalog, CatalogEntry, CatalogSource, ServiceId};
pub use deepseek::DeepSeekAdapter;
pub use opencode::{OpenCodeAdapter, OpenCodeSurface};
