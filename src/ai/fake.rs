//! Deterministic in-process AI service used by tests and prototypes.
use super::{
    AiRequest, AiResult, AiService, FinishReason, ServiceCapabilities, ServiceError, ServiceEvents,
    ToolCall, Usage,
};
use crate::json::JsonValue;
use std::{thread, time::Duration};

/// One scripted round of tool calls.
#[derive(Debug, Clone)]
pub struct FakeRound {
    /// Tool calls issued in this round, in order.
    pub calls: Vec<(String, JsonValue)>,
}
impl FakeRound {
    /// A round issuing the given calls.
    pub fn calls(calls: Vec<(String, JsonValue)>) -> Self {
        Self { calls }
    }
}

/// A deterministic service: scripted tool rounds followed by one final answer.
#[derive(Debug, Clone)]
pub struct FakeAiService {
    rounds: Vec<FakeRound>,
    answer: String,
    model: String,
    delay: Option<Duration>,
    failure: Option<ServiceError>,
    capabilities: ServiceCapabilities,
}
impl FakeAiService {
    /// A service that immediately answers without tools.
    pub fn answer(text: impl Into<String>) -> Self {
        Self {
            rounds: Vec::new(),
            answer: text.into(),
            model: "fake-model".to_owned(),
            delay: None,
            failure: None,
            capabilities: ServiceCapabilities::tool_capable("fake"),
        }
    }
    /// A service that issues the given rounds before answering.
    pub fn with_rounds(rounds: Vec<FakeRound>) -> Self {
        Self {
            rounds,
            ..Self::answer("done")
        }
    }
    /// Override the reported model identity.
    pub fn model(mut self, model: impl Into<String>) -> Self {
        self.model = model.into();
        self
    }
    /// Override the advertised adapter capabilities.
    pub fn capabilities(mut self, capabilities: ServiceCapabilities) -> Self {
        self.capabilities = capabilities;
        self
    }
    /// Wait before doing any work, to exercise cancellation while suspended.
    pub fn delay(mut self, delay: Duration) -> Self {
        self.delay = Some(delay);
        self
    }
    /// Fail the whole run with the given error.
    pub fn failing(error: ServiceError) -> Self {
        Self {
            failure: Some(error),
            ..Self::answer("unused")
        }
    }
}

impl AiService for FakeAiService {
    fn capabilities(&self) -> ServiceCapabilities {
        self.capabilities.clone()
    }

    fn run(
        &mut self,
        request: AiRequest,
        events: &mut dyn ServiceEvents,
    ) -> std::result::Result<AiResult, ServiceError> {
        if let Some(error) = &self.failure {
            return Err(error.clone());
        }
        if let Some(delay) = self.delay {
            thread::sleep(delay);
        }
        let mut call_id = 1_u64;
        for (round, scripted) in self.rounds.iter().enumerate() {
            if round as u32 >= request.max_turns {
                return Ok(AiResult {
                    text: String::new(),
                    finish_reason: FinishReason::Length,
                    model: self.model.clone(),
                    usage: Usage::default(),
                    request_id: None,
                });
            }
            for (name, arguments) in &scripted.calls {
                events.call_tool(ToolCall {
                    id: format!("call-{call_id}"),
                    name: name.clone(),
                    arguments: arguments.clone(),
                })?;
                call_id += 1;
            }
        }
        Ok(AiResult {
            text: self.answer.clone(),
            finish_reason: FinishReason::Stop,
            model: self.model.clone(),
            usage: Usage::default(),
            request_id: None,
        })
    }
}
