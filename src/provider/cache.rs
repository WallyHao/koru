//! Versioned, per-service catalog cache under the XDG cache directory.
//!
//! The cache is disposable: an unreadable, malformed, or unsupported cache file
//! is ignored and refetched, never fatal. Writes are atomic and serialized by the
//! shared file lock. Refreshing one service replaces only that service and leaves
//! its previous entries intact when the fetch fails.
use super::catalog::{Catalog, CatalogEntry, CatalogSource, ServiceId, parse_service_catalog};
use crate::{
    error::{ErrorCode, KoruError, Result},
    json::{JsonLimits, JsonValue, emit, parse},
    persist::{FileLock, write_atomic},
};
use std::path::{Path, PathBuf};

/// Cache document schema version.
pub const CACHE_SCHEMA_VERSION: u32 = 1;

/// A loaded cache document.
#[derive(Debug, Clone)]
pub struct CachedCatalog {
    /// Parsed entries.
    pub entries: Vec<CatalogEntry>,
    /// Unix seconds when the entries were fetched.
    pub fetched_at: u64,
}

/// The per-service catalog cache.
#[derive(Debug, Clone)]
pub struct CatalogCache {
    dir: PathBuf,
}
impl CatalogCache {
    /// Root the cache under the given Koru cache directory.
    pub fn new(cache_root: impl AsRef<Path>) -> Self {
        Self {
            dir: cache_root.as_ref().join("catalog"),
        }
    }
    /// The cache file for one service.
    pub fn path(&self, service: ServiceId) -> PathBuf {
        self.dir.join(format!("{}.json", service.name()))
    }

    /// Load one service's cache; corrupt or unsupported data yields `None`.
    pub fn load(&self, service: ServiceId) -> Option<CachedCatalog> {
        let text = std::fs::read_to_string(self.path(service)).ok()?;
        let document = parse(text.as_bytes(), &JsonLimits::default()).ok()?;
        Self::decode(service, &document).ok()
    }

    /// Store the raw fetched document atomically under the shared lock.
    pub fn store(&self, service: ServiceId, document: &JsonValue, fetched_at: u64) -> Result<()> {
        let payload = JsonValue::Object(
            [
                (
                    "schema_version".to_owned(),
                    JsonValue::Integer(i64::from(CACHE_SCHEMA_VERSION)),
                ),
                (
                    "fetched_at".to_owned(),
                    JsonValue::Integer(fetched_at as i64),
                ),
                ("document".to_owned(), document.clone()),
            ]
            .into_iter()
            .collect(),
        );
        let path = self.path(service);
        let _lock = FileLock::acquire(&path)?;
        write_atomic(&path, emit(&payload).as_bytes())
    }

    /// Fetch, cache, and replace one service; on failure load the cache instead.
    ///
    /// The error is returned even when cached entries were loaded, so the caller
    /// can report the affected service.
    pub fn refresh(
        &self,
        catalog: &mut Catalog,
        service: ServiceId,
        source: &dyn CatalogSource,
        fetched_at: u64,
    ) -> Result<()> {
        match source.fetch(service) {
            Ok(document) => {
                let entries = parse_service_catalog(service, &document, fetched_at, "live")?;
                self.store(service, &document, fetched_at)?;
                catalog.replace_service(service, entries);
                Ok(())
            }
            Err(error) => {
                if let Some(cached) = self.load(service) {
                    catalog.replace_service(service, cached.entries);
                }
                Err(error)
            }
        }
    }

    /// Whether a model is present in cache; `None` when nothing is cached.
    pub fn cached_model(&self, service: ServiceId, model: &str) -> Option<bool> {
        self.load(service)
            .map(|cached| cached.entries.iter().any(|entry| entry.id == model))
    }

    fn decode(service: ServiceId, document: &JsonValue) -> Result<CachedCatalog> {
        let JsonValue::Object(root) = document else {
            return Err(cache_error("cache must be a JSON object"));
        };
        match root.get("schema_version") {
            Some(JsonValue::Integer(version)) if *version == i64::from(CACHE_SCHEMA_VERSION) => {}
            Some(JsonValue::Integer(_)) | None => {
                return Err(cache_error("unsupported cache schema version"));
            }
            Some(_) => return Err(cache_error("cache `schema_version` must be an integer")),
        }
        let fetched_at = match root.get("fetched_at") {
            Some(JsonValue::Integer(value)) if *value >= 0 => *value as u64,
            _ => {
                return Err(cache_error(
                    "cache `fetched_at` must be a nonnegative integer",
                ));
            }
        };
        let payload = root
            .get("document")
            .ok_or_else(|| cache_error("cache is missing its `document`"))?;
        let entries = parse_service_catalog(service, payload, fetched_at, "cache")?;
        Ok(CachedCatalog {
            entries,
            fetched_at,
        })
    }
}

fn cache_error(detail: impl Into<String>) -> KoruError {
    KoruError::new(
        ErrorCode::Validation,
        format!("invalid catalog cache: {}", detail.into()),
    )
}
