//! DeepSeek adapter: Chat Completions conversation plus model listing.
use super::{
    catalog::{CatalogSource, ServiceId},
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
use std::sync::Arc;

/// DeepSeek API base URL.
pub const API_BASE: &str = "https://api.deepseek.com";
/// Chat Completions path.
pub const CHAT_PATH: &str = "/chat/completions";
/// Model listing path.
pub const MODELS_PATH: &str = "/models";

/// The DeepSeek provider adapter.
pub struct DeepSeekAdapter {
    transport: Arc<dyn Transport>,
    credential: String,
    model: String,
    effort: Option<String>,
}
impl DeepSeekAdapter {
    /// Build an adapter for one fixed model selection.
    pub fn new(
        transport: Arc<dyn Transport>,
        credential: impl Into<String>,
        model: impl Into<String>,
        effort: Option<String>,
    ) -> Self {
        Self {
            transport,
            credential: credential.into(),
            model: model.into(),
            effort,
        }
    }
    fn secrets(&self) -> [&str; 1] {
        [self.credential.as_str()]
    }
}
impl std::fmt::Debug for DeepSeekAdapter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DeepSeekAdapter")
            .field("model", &self.model)
            .field("effort", &self.effort)
            .field("credential", &"[redacted]")
            .finish()
    }
}
impl AiService for DeepSeekAdapter {
    fn capabilities(&self) -> ServiceCapabilities {
        ServiceCapabilities {
            provider: "deepseek".to_owned(),
            tools: true,
            structured_output: false,
            schema_keywords: SUPPORTED_KEYWORDS.into_iter().collect(),
            variants: Vec::new(),
            sessions: false,
        }
    }

    fn run(
        &mut self,
        request: AiRequest,
        events: &mut dyn ServiceEvents,
    ) -> std::result::Result<AiResult, ServiceError> {
        let endpoint = ChatEndpoint {
            url: format!("{API_BASE}{CHAT_PATH}"),
            model: self.model.clone(),
            effort: self.effort.clone(),
            credential: self.credential.clone(),
            session: None,
        };
        chat::run_chat(self.transport.as_ref(), &endpoint, request, events)
    }
}
impl CatalogSource for DeepSeekAdapter {
    fn fetch(&self, service: ServiceId) -> Result<JsonValue> {
        if service != ServiceId::DeepSeek {
            return Err(KoruError::new(
                ErrorCode::UnsupportedCapability,
                "the DeepSeek adapter serves only the deepseek service",
            ));
        }
        let request =
            chat::authorized_get(format!("{API_BASE}{MODELS_PATH}"), &self.credential, None);
        let response = self
            .transport
            .send(&request)
            .map_err(|error| http::transport_error(error, &self.secrets()))
            .map_err(service_error)?;
        let response = http::require_success(response, &self.secrets()).map_err(service_error)?;
        http::json_body(&response).map_err(service_error)
    }
}

fn service_error(error: ServiceError) -> KoruError {
    KoruError::new(ErrorCode::ProviderFailure, error.message)
}
