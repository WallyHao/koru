//! Catalog parsing, service identity, and refresh contracts.
use koru::{
    error::ErrorCode,
    json::JsonValue,
    provider::catalog::{
        Catalog, CatalogSource, FixtureCatalogSource, ServiceId, UnavailableCatalogSource,
        parse_service_catalog,
    },
};

fn obj(pairs: impl IntoIterator<Item = (&'static str, JsonValue)>) -> JsonValue {
    JsonValue::Object(
        pairs
            .into_iter()
            .map(|(key, value)| (key.to_owned(), value))
            .collect(),
    )
}
fn arr(items: Vec<JsonValue>) -> JsonValue {
    JsonValue::Array(items)
}
fn s(value: &str) -> JsonValue {
    JsonValue::String(value.to_owned())
}
fn model(id: &str) -> JsonValue {
    obj([("id", s(id))])
}
fn document(models: impl IntoIterator<Item = JsonValue>) -> JsonValue {
    obj([("data", JsonValue::Array(models.into_iter().collect()))])
}

#[test]
fn service_identities_are_stable() {
    assert_eq!(ServiceId::parse("deepseek"), Some(ServiceId::DeepSeek));
    assert_eq!(ServiceId::parse("opencode-go"), Some(ServiceId::OpenCodeGo));
    assert_eq!(ServiceId::parse("opencode"), Some(ServiceId::OpenCode));
    assert_eq!(ServiceId::parse("other"), None);
    assert_eq!(
        ServiceId::DeepSeek.credential_variable(),
        "DEEPSEEK_API_KEY"
    );
    assert_eq!(
        ServiceId::OpenCodeGo.credential_variable(),
        "OPENCODE_API_KEY"
    );
}

#[test]
fn parses_full_metadata_and_ignores_unknown_fields() {
    let doc = document(vec![obj([
        ("id", s("deepseek-chat")),
        ("name", s("DeepSeek Chat")),
        ("context_length", JsonValue::Integer(65536)),
        (
            "capabilities",
            obj([
                ("tools", JsonValue::Bool(true)),
                ("structured_output", JsonValue::Bool(false)),
                ("unknown", JsonValue::Bool(true)),
            ]),
        ),
        ("variants", arr(vec![s("high"), s("low")])),
        ("future_field", JsonValue::Bool(true)),
    ])]);
    let entries = parse_service_catalog(ServiceId::DeepSeek, &doc, 42, "fixture").unwrap();
    assert_eq!(entries.len(), 1);
    let entry = &entries[0];
    assert_eq!(entry.id, "deepseek-chat");
    assert_eq!(entry.display_name, "DeepSeek Chat");
    assert_eq!(entry.context_length, Some(65536));
    assert_eq!(entry.supports_tools, Some(true));
    assert_eq!(entry.supports_structured_output, Some(false));
    assert_eq!(entry.variants, vec!["high".to_owned(), "low".to_owned()]);
    assert_eq!(entry.fetched_at, 42);
}

#[test]
fn absent_capabilities_stay_unknown() {
    let doc = document(vec![model("m")]);
    let entries = parse_service_catalog(ServiceId::OpenCode, &doc, 1, "fixture").unwrap();
    assert_eq!(entries[0].display_name, "m");
    assert_eq!(entries[0].supports_tools, None);
    assert_eq!(entries[0].supports_structured_output, None);
    assert!(entries[0].variants.is_empty());
}

#[test]
fn rejects_malformed_catalogs() {
    for doc in [
        JsonValue::Bool(true),
        obj([("data", JsonValue::Bool(true))]),
        obj([("data", arr(vec![JsonValue::Bool(true)]))]),
        document(vec![obj([("name", s("missing id"))])]),
        document(vec![obj([("id", JsonValue::Integer(1))])]),
        document(vec![obj([("id", s("m")), ("context_length", s("big"))])]),
        document(vec![obj([
            ("id", s("m")),
            ("capabilities", JsonValue::Bool(true)),
        ])]),
        document(vec![obj([
            ("id", s("m")),
            ("capabilities", obj([("tools", s("yes"))])),
        ])]),
        document(vec![obj([("id", s("m"))]), obj([("id", s("m"))])]),
        document(vec![obj([
            ("id", s("m")),
            ("variants", arr(vec![s("a"), s("a")])),
        ])]),
    ] {
        assert_eq!(
            parse_service_catalog(ServiceId::DeepSeek, &doc, 1, "fixture")
                .unwrap_err()
                .code(),
            ErrorCode::Validation
        );
    }
}

#[test]
fn refresh_replaces_only_the_requested_service_and_survives_failure() {
    let source = FixtureCatalogSource::new()
        .with(
            ServiceId::DeepSeek,
            document(vec![model("ds-1"), model("ds-2")]),
        )
        .with(ServiceId::OpenCode, document(vec![model("oc-1")]));
    let mut catalog = Catalog::default();
    catalog.refresh(ServiceId::DeepSeek, &source, 100).unwrap();
    catalog.refresh(ServiceId::OpenCode, &source, 100).unwrap();
    assert_eq!(catalog.entries(ServiceId::DeepSeek).len(), 2);
    assert_eq!(catalog.entries(ServiceId::OpenCode).len(), 1);

    let bad = FixtureCatalogSource::new()
        .with(ServiceId::DeepSeek, obj([("data", JsonValue::Bool(false))]));
    assert_eq!(
        catalog
            .refresh(ServiceId::DeepSeek, &bad, 200)
            .unwrap_err()
            .code(),
        ErrorCode::Validation
    );
    assert_eq!(catalog.entries(ServiceId::DeepSeek).len(), 2);
    assert_eq!(
        catalog
            .model(ServiceId::DeepSeek, "ds-1")
            .unwrap()
            .fetched_at,
        100
    );
}

#[test]
fn unavailable_source_reports_unsupported_capability() {
    let mut catalog = Catalog::default();
    let error = catalog
        .refresh(ServiceId::OpenCodeGo, &UnavailableCatalogSource, 0)
        .unwrap_err();
    assert_eq!(error.code(), ErrorCode::UnsupportedCapability);
    assert!(catalog.entries(ServiceId::OpenCodeGo).is_empty());
}

#[test]
fn fixture_source_fetches_registered_services_only() {
    let source = FixtureCatalogSource::new().with(ServiceId::DeepSeek, document(vec![model("ds")]));
    assert!(source.fetch(ServiceId::DeepSeek).is_ok());
    assert_eq!(
        source.fetch(ServiceId::OpenCode).unwrap_err().code(),
        ErrorCode::UnsupportedCapability
    );
}
