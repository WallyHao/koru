//! Bounded model catalogs per provider service.
//!
//! Catalogs are advisory metadata, never a grant: a missing capability field is
//! unknown and MUST NOT be treated as supported. Parsing is strict about known
//! field types and bounds and ignores unrecognized provider fields so catalogs can
//! evolve without breaking selection. Refreshing one service replaces only that
//! service's entries and leaves previous entries intact on failure.
use crate::{
    credentials::{DEEPSEEK_API_KEY, OPENCODE_API_KEY},
    error::{ErrorCode, KoruError, Result},
    json::JsonValue,
};
use std::collections::BTreeMap;

/// Maximum models retained for one service.
pub const MAX_CATALOG_ENTRIES: usize = 1000;
/// Maximum bytes in a model id, display name, or variant.
pub const MAX_LABEL_BYTES: usize = 256;
/// Maximum declared variants for one model.
pub const MAX_VARIANTS: usize = 16;

/// A provider service with its own identity, credentials, and catalog.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ServiceId {
    /// DeepSeek's OpenAI-compatible API.
    DeepSeek,
    /// OpenCode Zen.
    OpenCode,
    /// OpenCode Go.
    OpenCodeGo,
}
impl ServiceId {
    /// Every implemented service, in display order.
    pub const ALL: [ServiceId; 3] = [Self::DeepSeek, Self::OpenCode, Self::OpenCodeGo];

    /// Stable identity used in configuration and model names.
    pub fn name(self) -> &'static str {
        match self {
            Self::DeepSeek => "deepseek",
            Self::OpenCode => "opencode",
            Self::OpenCodeGo => "opencode-go",
        }
    }
    /// Parse a service identity; unknown names are rejected.
    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|service| service.name() == name)
    }
    /// Environment variable holding this service's key.
    pub fn credential_variable(self) -> &'static str {
        match self {
            Self::DeepSeek => DEEPSEEK_API_KEY,
            Self::OpenCode | Self::OpenCodeGo => OPENCODE_API_KEY,
        }
    }
    /// Documented catalog endpoint, used once a transport exists.
    pub fn catalog_endpoint(self) -> &'static str {
        match self {
            Self::DeepSeek => "https://api.deepseek.com/models",
            Self::OpenCode => "https://opencode.ai/zen/v1/models",
            Self::OpenCodeGo => "https://opencode.ai/zen/go/v1/models",
        }
    }
}

/// One catalog model and its advertised metadata.
#[derive(Debug, Clone, PartialEq)]
pub struct CatalogEntry {
    /// Model identity within the service.
    pub id: String,
    /// Owning service.
    pub service: ServiceId,
    /// Human-readable name; defaults to the id.
    pub display_name: String,
    /// Advertised context window, when present.
    pub context_length: Option<u64>,
    /// Advertised tool support; absent means unknown.
    pub supports_tools: Option<bool>,
    /// Advertised structured-output support; absent means unknown.
    pub supports_structured_output: Option<bool>,
    /// Advertised effort variants.
    pub variants: Vec<String>,
    /// Where the entry was read from.
    pub source: String,
    /// Unix seconds when the entry was fetched.
    pub fetched_at: u64,
}

/// Variants a service advertises before adapters and catalogs provide their own.
///
/// Provisional until real adapters report capabilities; catalog metadata will
/// replace this once a transport exists.
pub fn declared_variants(service: ServiceId) -> &'static [&'static str] {
    match service {
        ServiceId::OpenCodeGo => &["high"],
        ServiceId::DeepSeek | ServiceId::OpenCode => &[],
    }
}

/// A source of bounded catalog documents.
pub trait CatalogSource {
    /// Fetch the raw document for one service.
    fn fetch(&self, service: ServiceId) -> Result<JsonValue>;
}

/// The production source used until a real transport is implemented.
#[derive(Debug, Default, Clone, Copy)]
pub struct UnavailableCatalogSource;
impl CatalogSource for UnavailableCatalogSource {
    fn fetch(&self, service: ServiceId) -> Result<JsonValue> {
        Err(KoruError::new(
            ErrorCode::UnsupportedCapability,
            format!(
                "no transport is implemented for {}; metadata endpoint {} is not queried",
                service.name(),
                service.catalog_endpoint()
            ),
        ))
    }
}

/// A deterministic source backed by fixed documents, for tests.
#[derive(Debug, Default, Clone)]
pub struct FixtureCatalogSource {
    documents: BTreeMap<ServiceId, JsonValue>,
}
impl FixtureCatalogSource {
    /// A source with no documents; every fetch fails as unsupported.
    pub fn new() -> Self {
        Self::default()
    }
    /// Register a document for one service.
    pub fn with(mut self, service: ServiceId, document: JsonValue) -> Self {
        self.documents.insert(service, document);
        self
    }
}
impl CatalogSource for FixtureCatalogSource {
    fn fetch(&self, service: ServiceId) -> Result<JsonValue> {
        self.documents.get(&service).cloned().ok_or_else(|| {
            KoruError::new(
                ErrorCode::UnsupportedCapability,
                format!("no fixture catalog for {}", service.name()),
            )
        })
    }
}

/// Parsed catalogs for the implemented services.
#[derive(Debug, Clone, Default)]
pub struct Catalog {
    services: BTreeMap<ServiceId, Vec<CatalogEntry>>,
}
impl Catalog {
    /// Entries for one service, or an empty slice.
    pub fn entries(&self, service: ServiceId) -> &[CatalogEntry] {
        self.services
            .get(&service)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }
    /// Look up one model by id.
    pub fn model(&self, service: ServiceId, id: &str) -> Option<&CatalogEntry> {
        self.entries(service).iter().find(|entry| entry.id == id)
    }
    /// Replace only one service's entries.
    pub fn replace_service(&mut self, service: ServiceId, entries: Vec<CatalogEntry>) {
        self.services.insert(service, entries);
    }
    /// Fetch, parse, and replace one service; failure leaves prior entries intact.
    pub fn refresh(
        &mut self,
        service: ServiceId,
        source: &dyn CatalogSource,
        fetched_at: u64,
    ) -> Result<()> {
        let document = source.fetch(service)?;
        let entries =
            parse_service_catalog(service, &document, fetched_at, service.catalog_endpoint())?;
        self.replace_service(service, entries);
        Ok(())
    }
}

/// Parse one service document into bounded entries.
pub fn parse_service_catalog(
    service: ServiceId,
    document: &JsonValue,
    fetched_at: u64,
    source: &str,
) -> Result<Vec<CatalogEntry>> {
    let JsonValue::Object(root) = document else {
        return Err(catalog_error("a catalog must be a JSON object"));
    };
    let data = root
        .get("data")
        .ok_or_else(|| catalog_error("a catalog needs a `data` array"))?;
    let JsonValue::Array(items) = data else {
        return Err(catalog_error("`data` must be an array"));
    };
    if items.len() > MAX_CATALOG_ENTRIES {
        return Err(catalog_error(format!(
            "a catalog may hold at most {MAX_CATALOG_ENTRIES} models"
        )));
    }
    let mut entries = Vec::with_capacity(items.len());
    let mut seen = std::collections::BTreeSet::new();
    for item in items {
        let entry = parse_entry(service, item, fetched_at, source)?;
        if !seen.insert(entry.id.clone()) {
            return Err(catalog_error(format!("duplicate model id {:?}", entry.id)));
        }
        entries.push(entry);
    }
    Ok(entries)
}

fn parse_entry(
    service: ServiceId,
    item: &JsonValue,
    fetched_at: u64,
    source: &str,
) -> Result<CatalogEntry> {
    let JsonValue::Object(fields) = item else {
        return Err(catalog_error("each catalog model must be an object"));
    };
    let id = label(fields.get("id"), "id")?;
    let display_name = match fields.get("name").or_else(|| fields.get("display_name")) {
        Some(value) => label(Some(value), "name")?,
        None => id.clone(),
    };
    let context_length = match fields
        .get("context_length")
        .or_else(|| fields.get("context_window"))
    {
        Some(JsonValue::Integer(value)) if *value >= 0 => Some(*value as u64),
        Some(_) => {
            return Err(catalog_error(
                "`context_length` must be a nonnegative integer",
            ));
        }
        None => None,
    };
    let (supports_tools, supports_structured_output) = match fields.get("capabilities") {
        Some(JsonValue::Object(capabilities)) => (
            optional_bool(capabilities.get("tools"), "capabilities.tools")?,
            optional_bool(
                capabilities.get("structured_output"),
                "capabilities.structured_output",
            )?,
        ),
        Some(_) => return Err(catalog_error("`capabilities` must be an object")),
        None => (None, None),
    };
    let variants = match fields.get("variants") {
        Some(JsonValue::Array(values)) => {
            if values.len() > MAX_VARIANTS {
                return Err(catalog_error(format!(
                    "a model may declare at most {MAX_VARIANTS} variants"
                )));
            }
            let mut variants = Vec::with_capacity(values.len());
            for value in values {
                let name = label(Some(value), "variant")?;
                if variants.contains(&name) {
                    return Err(catalog_error(format!("duplicate variant {name:?}")));
                }
                variants.push(name);
            }
            variants
        }
        Some(_) => return Err(catalog_error("`variants` must be an array")),
        None => Vec::new(),
    };
    Ok(CatalogEntry {
        id,
        service,
        display_name,
        context_length,
        supports_tools,
        supports_structured_output,
        variants,
        source: source.to_owned(),
        fetched_at,
    })
}

fn label(value: Option<&JsonValue>, field: &str) -> Result<String> {
    match value {
        Some(JsonValue::String(text)) if !text.is_empty() && text.len() <= MAX_LABEL_BYTES => {
            Ok(text.clone())
        }
        Some(JsonValue::String(_)) => Err(catalog_error(format!(
            "`{field}` must be 1..={MAX_LABEL_BYTES} bytes"
        ))),
        Some(_) => Err(catalog_error(format!("`{field}` must be a string"))),
        None => Err(catalog_error(format!(
            "a catalog model is missing `{field}`"
        ))),
    }
}

fn optional_bool(value: Option<&JsonValue>, field: &str) -> Result<Option<bool>> {
    match value {
        Some(JsonValue::Bool(flag)) => Ok(Some(*flag)),
        Some(_) => Err(catalog_error(format!("`{field}` must be a boolean"))),
        None => Ok(None),
    }
}

fn catalog_error(detail: impl Into<String>) -> KoruError {
    KoruError::new(
        ErrorCode::Validation,
        format!("invalid catalog: {}", detail.into()),
    )
}
