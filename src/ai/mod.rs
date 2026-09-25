//! Koru-owned AI and tool contracts; provider adapters stay behind this boundary.
//!
//! Services never see Lua or provider types. One agent request is bounded by the
//! caller's [`max_turns`](AiRequest::max_turns); the service dispatches model
//! tool calls through [`ServiceEvents`], which the VM owner implements.
pub mod fake;

use crate::{
    json::JsonValue,
    schema::{JsonSchema, SUPPORTED_KEYWORDS},
};
use std::{
    collections::BTreeSet,
    sync::mpsc::{Receiver, SyncSender},
};

/// Adapter-owned facts about what a service can do for one run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceCapabilities {
    /// Provider identity used in diagnostics.
    pub provider: String,
    /// Whether the service can dispatch model tool calls.
    pub tools: bool,
    /// Whether the service supports structured output.
    pub structured_output: bool,
    /// Validation keywords the service accepts in tool schemas.
    pub schema_keywords: BTreeSet<&'static str>,
    /// Effort variants the service accepts.
    pub variants: Vec<String>,
    /// Whether conversations use a stable session header.
    pub sessions: bool,
}
impl ServiceCapabilities {
    /// A tool-capable service that accepts the full Koru schema subset.
    pub fn tool_capable(provider: impl Into<String>) -> Self {
        Self {
            provider: provider.into(),
            tools: true,
            structured_output: false,
            schema_keywords: SUPPORTED_KEYWORDS.into_iter().collect(),
            variants: Vec::new(),
            sessions: false,
        }
    }
    /// Whether every keyword the schema uses is supported.
    pub fn supports_keywords(&self, keywords: &BTreeSet<&'static str>) -> bool {
        keywords.is_subset(&self.schema_keywords)
    }
}

/// One bounded agent request.
#[derive(Debug, Clone)]
pub struct AiRequest {
    /// The user or workflow prompt.
    pub prompt: String,
    /// The immutable tool subset offered to this agent run.
    pub tools: Vec<ToolSpec>,
    /// Maximum model turns for this run.
    pub max_turns: u32,
}

/// An immutable tool description offered to a model.
#[derive(Debug, Clone)]
pub struct ToolSpec {
    /// Unique tool name.
    pub name: String,
    /// Human-readable description.
    pub description: String,
    /// Compiled JSON schema for the arguments.
    pub parameters: JsonSchema,
}

/// A tool invocation requested by a service.
#[derive(Debug, Clone)]
pub struct ToolCall {
    /// Provider-assigned call identity, unique within the run.
    pub id: u64,
    /// Declared tool name.
    pub name: String,
    /// JSON arguments.
    pub arguments: JsonValue,
}

/// The result returned to a service for one tool call.
#[derive(Debug, Clone)]
pub struct ToolResult {
    /// JSON content produced by the tool.
    pub content: JsonValue,
}

/// Why an agent run ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FinishReason {
    /// The model produced a final answer.
    Stop,
    /// A model or Koru turn limit was reached.
    Length,
    /// The run ended on tool requests that were not continued.
    ToolCalls,
    /// A provider content filter stopped generation.
    ContentFilter,
    /// A provider-specific reason.
    Other(String),
}

/// Token usage; missing provider data stays explicitly unknown.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Usage {
    /// Prompt tokens, when reported.
    pub input_tokens: Option<u64>,
    /// Generated tokens, when reported.
    pub output_tokens: Option<u64>,
}

/// A completed agent run.
#[derive(Debug, Clone)]
pub struct AiResult {
    /// Final model text.
    pub text: String,
    /// Normalized finish reason.
    pub finish_reason: FinishReason,
    /// Selected model identity.
    pub model: String,
    /// Available token usage.
    pub usage: Usage,
    /// Provider request identifiers, when available.
    pub request_id: Option<String>,
}

/// The category of a service failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceErrorKind {
    /// A provider or transport failure.
    Provider,
    /// A tool failed while producing its result.
    Tool,
    /// The owner stopped the run.
    Cancelled,
}

/// A classified service failure.
#[derive(Debug, Clone)]
pub struct ServiceError {
    /// Failure category.
    pub kind: ServiceErrorKind,
    /// Bounded, provider-neutral message.
    pub message: String,
}
impl ServiceError {
    /// A provider failure.
    pub fn provider(message: impl Into<String>) -> Self {
        Self {
            kind: ServiceErrorKind::Provider,
            message: message.into(),
        }
    }
    /// A tool failure.
    pub fn tool(message: impl Into<String>) -> Self {
        Self {
            kind: ServiceErrorKind::Tool,
            message: message.into(),
        }
    }
    /// A cancellation observed by the service.
    pub fn cancelled(message: impl Into<String>) -> Self {
        Self {
            kind: ServiceErrorKind::Cancelled,
            message: message.into(),
        }
    }
}

/// The worker side of one agent run; implemented by provider adapters and fakes.
pub trait AiService: Send + 'static {
    /// Immutable adapter capabilities for this run.
    fn capabilities(&self) -> ServiceCapabilities;

    /// Run one bounded agent loop, dispatching tools through `events`.
    fn run(
        &mut self,
        request: AiRequest,
        events: &mut dyn ServiceEvents,
    ) -> std::result::Result<AiResult, ServiceError>;
}

/// Owner-provided operations available to a running service.
pub trait ServiceEvents {
    /// Invoke one tool on the VM owner and wait for its result.
    fn call_tool(&mut self, call: ToolCall) -> std::result::Result<ToolResult, ServiceError>;
}

/// An event sent from a service worker to the VM owner.
pub(crate) enum Event {
    /// The service asks the owner to run one tool.
    ToolCall(ToolCall),
    /// The service finished the run.
    Finished(std::result::Result<AiResult, ServiceError>),
}

/// A reply sent from the VM owner back to a service worker.
pub(crate) enum Reply {
    /// The tool call outcome.
    Result(std::result::Result<ToolResult, ServiceError>),
}

/// The channel-backed [`ServiceEvents`] used across the worker boundary.
pub(crate) struct ChannelEvents {
    /// Bounded owner-bound event queue.
    pub(crate) events: SyncSender<Event>,
    /// Bounded service-bound reply queue.
    pub(crate) replies: Receiver<Reply>,
}
impl ServiceEvents for ChannelEvents {
    fn call_tool(&mut self, call: ToolCall) -> std::result::Result<ToolResult, ServiceError> {
        self.events
            .send(Event::ToolCall(call))
            .map_err(|_| ServiceError::cancelled("the VM owner stopped the run"))?;
        match self.replies.recv() {
            Ok(Reply::Result(result)) => result,
            Err(_) => Err(ServiceError::cancelled("the VM owner stopped the run")),
        }
    }
}
