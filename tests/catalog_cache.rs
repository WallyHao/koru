//! Catalog cache: atomic persistence, corruption tolerance, and refresh rules.
use koru::{
    error::ErrorCode,
    json::{JsonLimits, JsonValue, parse},
    provider::{Catalog, CatalogCache, CatalogSource, ServiceId, catalog::FixtureCatalogSource},
};
use std::fs;

fn document(ids: &[&str]) -> JsonValue {
    let entries = ids
        .iter()
        .map(|id| format!(r#"{{"id":"{id}"}}"#))
        .collect::<Vec<_>>()
        .join(",");
    parse(
        format!(r#"{{"data":[{entries}]}}"#).as_bytes(),
        &JsonLimits::default(),
    )
    .unwrap()
}

#[test]
fn stores_and_loads_per_service() {
    let dir = tempfile::tempdir().unwrap();
    let cache = CatalogCache::new(dir.path());
    cache
        .store(ServiceId::DeepSeek, &document(&["a", "b"]), 100)
        .unwrap();
    let cached = cache.load(ServiceId::DeepSeek).unwrap();
    assert_eq!(cached.fetched_at, 100);
    assert_eq!(cached.entries.len(), 2);
    assert_eq!(cached.entries[0].id, "a");
    assert!(cache.load(ServiceId::OpenCodeGo).is_none());

    let files = fs::read_dir(cache.path(ServiceId::DeepSeek).parent().unwrap())
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect::<Vec<_>>();
    assert_eq!(
        files.len(),
        1,
        "only the cache file should remain: {files:?}"
    );
}

#[test]
fn corrupt_or_unsupported_cache_is_ignored() {
    let dir = tempfile::tempdir().unwrap();
    let cache = CatalogCache::new(dir.path());
    let path = cache.path(ServiceId::DeepSeek);
    fs::create_dir_all(path.parent().unwrap()).unwrap();

    fs::write(&path, "not json").unwrap();
    assert!(cache.load(ServiceId::DeepSeek).is_none());

    fs::write(
        &path,
        r#"{"schema_version":9,"fetched_at":1,"document":{"data":[]}}"#,
    )
    .unwrap();
    assert!(cache.load(ServiceId::DeepSeek).is_none());

    fs::write(
        &path,
        r#"{"schema_version":1,"fetched_at":1,"document":{"data":[{"id":"a"}]}}"#,
    )
    .unwrap();
    assert!(cache.load(ServiceId::DeepSeek).is_some());
}

#[test]
fn refresh_replaces_only_the_target_and_survives_failure() {
    let dir = tempfile::tempdir().unwrap();
    let cache = CatalogCache::new(dir.path());
    let source = FixtureCatalogSource::new()
        .with(ServiceId::DeepSeek, document(&["a", "b"]))
        .with(ServiceId::OpenCode, document(&["c"]));
    let mut catalog = Catalog::default();
    cache
        .refresh(&mut catalog, ServiceId::DeepSeek, &source, 100)
        .unwrap();
    cache
        .refresh(&mut catalog, ServiceId::OpenCode, &source, 100)
        .unwrap();
    assert_eq!(catalog.entries(ServiceId::DeepSeek).len(), 2);
    assert_eq!(catalog.entries(ServiceId::OpenCode).len(), 1);

    let failing = FixtureCatalogSource::new();
    let error = cache
        .refresh(&mut catalog, ServiceId::DeepSeek, &failing, 200)
        .unwrap_err();
    assert_eq!(error.code(), ErrorCode::UnsupportedCapability);
    assert_eq!(catalog.entries(ServiceId::DeepSeek).len(), 2);
    assert_eq!(
        catalog.entries(ServiceId::DeepSeek)[0].fetched_at,
        100,
        "cached entries are reused after a failed refresh"
    );
}

#[test]
fn cached_model_reports_absent_unknown_and_present() {
    let dir = tempfile::tempdir().unwrap();
    let cache = CatalogCache::new(dir.path());
    assert_eq!(cache.cached_model(ServiceId::DeepSeek, "a"), None);
    let source = FixtureCatalogSource::new().with(ServiceId::DeepSeek, document(&["a"]));
    let mut catalog = Catalog::default();
    cache
        .refresh(&mut catalog, ServiceId::DeepSeek, &source, 1)
        .unwrap();
    assert_eq!(cache.cached_model(ServiceId::DeepSeek, "a"), Some(true));
    assert_eq!(cache.cached_model(ServiceId::DeepSeek, "z"), Some(false));
}

#[test]
fn refresh_without_a_cache_returns_the_fetch_error() {
    let dir = tempfile::tempdir().unwrap();
    let cache = CatalogCache::new(dir.path());
    let mut catalog = Catalog::default();
    let error = cache
        .refresh(
            &mut catalog,
            ServiceId::OpenCodeGo,
            &FixtureCatalogSource::new(),
            1,
        )
        .unwrap_err();
    assert_eq!(error.code(), ErrorCode::UnsupportedCapability);
    assert!(catalog.entries(ServiceId::OpenCodeGo).is_empty());
}

#[test]
fn sources_are_only_queried_for_their_own_service() {
    let source = FixtureCatalogSource::new().with(ServiceId::DeepSeek, document(&["a"]));
    assert!(source.fetch(ServiceId::DeepSeek).is_ok());
    assert!(source.fetch(ServiceId::OpenCode).is_err());
}
