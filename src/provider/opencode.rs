//! OpenCode Zen and Go adapters sharing one Chat Completions mapping.
//!
//! Zen and Go keep separate identities, endpoints, and catalogs. Both use the
//! OpenCode credential; Go additionally sends a stable conversation session
//! header on every request, including tool-loop turns.
use super::{
    catalog::{CatalogSource, ServiceId, declared_variants},
    chat::{self, ChatEndpoint},
    http,
};
use crate::{
    ai::{AiRequest, AiResult, AiService, ServiceCapabilities, ServiceError, ServiceEvents},
    error::{ErrorCode, KoruError, Result},
    json::JsonValue,
    schema::SUPPORTED_KEYWORDS,
    transport::Transport,
};
use sha2::{Digest, Sha256};
use std::{
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};

/// Header carrying the conversation session for OpenCode Go.
pub const SESSION_HEADER: &str = "x-opencode-session";
/// OpenCode Zen API base URL.
pub const ZEN_BASE: &str = "https://opencode.ai/zen/v1";
/// OpenCode Go API base URL.
pub const GO_BASE: &str = "https://opencode.ai/zen/go/v1";
/// Chat Completions path below either base.
pub const CHAT_PATH: &str = "/chat/completions";
/// Model listing path below either base.
pub const MODELS_PATH: &str = "/models";

/// Which OpenCode surface an adapter talks to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpenCodeSurface {
    /// OpenCode Zen.
    Zen,
    /// OpenCode Go.
    Go,
}
impl OpenCodeSurface {
    /// The service identity this surface serves.
    pub fn service(self) -> ServiceId {
        match self {
            Self::Zen => ServiceId::OpenCode,
            Self::Go => ServiceId::OpenCodeGo,
        }
    }
    fn base(self) -> &'static str {
        match self {
            Self::Zen => ZEN_BASE,
            Self::Go => GO_BASE,
        }
    }
    fn uses_sessions(self) -> bool {
        matches!(self, Self::Go)
    }
}

/// An OpenCode provider adapter for one surface and model selection.
pub struct OpenCodeAdapter {
    surface: OpenCodeSurface,
    transport: Arc<dyn Transport>,
    credential: String,
    model: String,
    effort: Option<String>,
    session: String,
}
impl OpenCodeAdapter {
    /// Build an adapter for one surface and fixed model selection.
    pub fn new(
        surface: OpenCodeSurface,
        transport: Arc<dyn Transport>,
        credential: impl Into<String>,
        model: impl Into<String>,
        effort: Option<String>,
    ) -> Self {
        Self {
            surface,
            transport,
            credential: credential.into(),
            model: model.into(),
            effort,
            session: new_session(),
        }
    }
    fn session(&self) -> Option<(&str, &str)> {
        self.surface
            .uses_sessions()
            .then_some((SESSION_HEADER, self.session.as_str()))
    }
    fn secrets(&self) -> [&str; 1] {
        [self.credential.as_str()]
    }
}
impl std::fmt::Debug for OpenCodeAdapter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpenCodeAdapter")
            .field("surface", &self.surface)
            .field("model", &self.model)
            .field("effort", &self.effort)
            .field("credential", &"[redacted]")
            .finish()
    }
}
impl AiService for OpenCodeAdapter {
    fn capabilities(&self) -> ServiceCapabilities {
        ServiceCapabilities {
            provider: self.surface.service().name().to_owned(),
            tools: true,
            structured_output: false,
            schema_keywords: SUPPORTED_KEYWORDS.into_iter().collect(),
            variants: declared_variants(self.surface.service())
                .iter()
                .map(|variant| (*variant).to_owned())
                .collect(),
            sessions: self.surface.uses_sessions(),
        }
    }

    fn run(
        &mut self,
        request: AiRequest,
        events: &mut dyn ServiceEvents,
    ) -> std::result::Result<AiResult, ServiceError> {
        let endpoint = ChatEndpoint {
            url: format!("{}{CHAT_PATH}", self.surface.base()),
            model: self.model.clone(),
            effort: self.effort.clone(),
            credential: self.credential.clone(),
            session: self
                .session()
                .map(|(name, value)| (name.to_owned(), value.to_owned())),
        };
        chat::run_chat(self.transport.as_ref(), &endpoint, request, events)
    }
}
impl CatalogSource for OpenCodeAdapter {
    fn fetch(&self, service: ServiceId) -> Result<JsonValue> {
        if service != self.surface.service() {
            return Err(KoruError::new(
                ErrorCode::UnsupportedCapability,
                format!(
                    "the {} adapter does not serve {}",
                    self.surface.service().name(),
                    service.name()
                ),
            ));
        }
        let request = chat::authorized_get(
            format!("{}{MODELS_PATH}", self.surface.base()),
            &self.credential,
            self.session(),
        );
        let response = self
            .transport
            .send(&request)
            .map_err(|error| http::transport_error(error, &self.secrets()))
            .map_err(service_error)?;
        let response = http::require_success(response, &self.secrets()).map_err(service_error)?;
        http::json_body(&response).map_err(service_error)
    }
}

/// A stable, unique conversation session identity.
fn new_session() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(1);
    let mut hasher = Sha256::new();
    hasher.update(std::process::id().to_le_bytes());
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_nanos())
        .unwrap_or(0);
    hasher.update(nanos.to_le_bytes());
    hasher.update(COUNTER.fetch_add(1, Ordering::Relaxed).to_le_bytes());
    let digest = hasher.finalize();
    let mut session = String::from("koru-");
    for byte in &digest[..16] {
        session.push_str(&format!("{byte:02x}"));
    }
    session
}

fn service_error(error: ServiceError) -> KoruError {
    KoruError::new(ErrorCode::ProviderFailure, error.message)
}
