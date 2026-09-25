//! Owner-side agent loop: run one service request and dispatch its tool calls.
use super::super::{command::ToolEntry, error, json};
use super::{AI_SERVICE_JOIN_GRACE, CHANNEL_CAPACITY, WAIT_TICK};
use crate::{
    ai::{
        AiRequest, AiResult, AiService, ChannelEvents, Event, Reply, ServiceCapabilities,
        ServiceError, ServiceErrorKind, ToolCall, ToolResult,
    },
    error::{ErrorCode, KoruError, Result},
    json::JsonLimits,
    runtime::{ExecutionContext, Resources},
};
use mlua::{Lua, Value};
use std::{
    cell::Cell,
    collections::BTreeMap,
    rc::Rc,
    sync::{
        Arc, Mutex,
        mpsc::{RecvTimeoutError, sync_channel},
    },
    time::Instant,
};

/// Run one bounded agent request, servicing tool calls until the service ends.
pub(super) fn drive_agent(
    service: &Arc<Mutex<Box<dyn AiService>>>,
    request: AiRequest,
    context: &ExecutionContext,
    tools: &BTreeMap<String, ToolEntry>,
    lua: &Lua,
    in_tool: &Rc<Cell<bool>>,
) -> Result<AiResult> {
    let capabilities = match service.lock() {
        Ok(service) => service.capabilities(),
        Err(_) => {
            return Err(error::coded(
                ErrorCode::ProviderFailure,
                "the AI service is unavailable",
            ));
        }
    };
    preflight(&request, &capabilities)?;
    context.reserve(
        Resources {
            model_requests: 1,
            bytes: request.prompt.len() as u64,
            ..Resources::ZERO
        },
        Instant::now(),
    )?;
    let (event_tx, event_rx) = sync_channel::<Event>(CHANNEL_CAPACITY);
    let (reply_tx, reply_rx) = sync_channel::<Reply>(CHANNEL_CAPACITY);
    let (done_tx, done_rx) = sync_channel::<()>(1);
    let worker_service = Arc::clone(service);
    let worker = std::thread::spawn(move || {
        let mut events = ChannelEvents {
            events: event_tx,
            replies: reply_rx,
        };
        let outcome = match worker_service.lock() {
            Ok(mut service) => service.run(request, &mut events),
            Err(_) => Err(ServiceError::provider("the AI service is unavailable")),
        };
        let _ = events.events.send(Event::Finished(outcome));
        let _ = done_tx.send(());
    });
    let mut terminal = None;
    let mut outcome = Err(ServiceError::provider("the AI service stopped"));
    loop {
        match event_rx.recv_timeout(WAIT_TICK) {
            Ok(Event::ToolCall(call)) => {
                if let Err(error) = context.reserve(
                    Resources {
                        tool_calls: 1,
                        ..Resources::ZERO
                    },
                    Instant::now(),
                ) {
                    terminal = Some(error);
                    break;
                }
                match dispatch_tool(tools, lua, in_tool, &call) {
                    Ok(result) => {
                        if reply_tx.send(Reply::Result(Ok(result))).is_err() {
                            break;
                        }
                    }
                    Err(DispatchError::Report(error)) => {
                        if reply_tx.send(Reply::Result(Err(error))).is_err() {
                            break;
                        }
                    }
                    Err(DispatchError::Stop(error)) => {
                        terminal = Some(error);
                        break;
                    }
                }
            }
            Ok(Event::Finished(result)) => {
                outcome = result;
                break;
            }
            Err(RecvTimeoutError::Timeout) => {
                if let Err(error) = context.ensure_active(Instant::now()) {
                    terminal = Some(error);
                    break;
                }
            }
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }
    drop(reply_tx);
    drop(event_rx);
    if let Some(error) = terminal {
        // Never block a terminal run on a service; the thread is detached and the
        // process reaps it when the closure eventually returns.
        return Err(error);
    }
    if done_rx.recv_timeout(AI_SERVICE_JOIN_GRACE).is_err() {
        return Err(error::coded(
            ErrorCode::Timeout,
            "the AI service did not stop within the shutdown grace",
        ));
    }
    let _ = worker.join();
    context.ensure_active(Instant::now())?;
    outcome.map_err(service_error)
}

/// Reject a request the adapter cannot serve before any budget is charged.
fn preflight(request: &AiRequest, capabilities: &ServiceCapabilities) -> Result<()> {
    if request.tools.is_empty() {
        return Ok(());
    }
    if !capabilities.tools {
        return Err(error::coded(
            ErrorCode::UnsupportedCapability,
            format!(
                "provider {:?} does not support tools",
                capabilities.provider
            ),
        ));
    }
    for spec in &request.tools {
        if !capabilities.supports_keywords(spec.parameters.keywords()) {
            return Err(error::coded(
                ErrorCode::UnsupportedCapability,
                format!(
                    "tool {:?} schema exceeds the capabilities of provider {:?}",
                    spec.name, capabilities.provider
                ),
            ));
        }
    }
    Ok(())
}

/// A tool-dispatch failure: report it to the service, or stop the whole run.
enum DispatchError {
    Report(ServiceError),
    Stop(KoruError),
}

fn dispatch_tool(
    tools: &BTreeMap<String, ToolEntry>,
    lua: &Lua,
    in_tool: &Rc<Cell<bool>>,
    call: &ToolCall,
) -> std::result::Result<ToolResult, DispatchError> {
    let Some(entry) = tools.get(&call.name) else {
        return Err(DispatchError::Report(ServiceError::tool(format!(
            "unknown tool {:?}",
            call.name
        ))));
    };
    if let Err(error) = entry.spec.parameters.validate(&call.arguments) {
        return Err(DispatchError::Stop(error::invalid(format!(
            "tool {:?} arguments {}",
            call.name,
            error.message()
        ))));
    }
    in_tool.set(true);
    let outcome = (|| -> std::result::Result<crate::json::JsonValue, ServiceError> {
        let argument = json::from_json(lua, &call.arguments)
            .map_err(|error| ServiceError::tool(error.message().to_owned()))?;
        let value = entry
            .callback
            .call::<Value>(argument)
            .map_err(|error| ServiceError::tool(error.to_string()))?;
        json::to_json(&value, &JsonLimits::default())
            .map_err(|error| ServiceError::tool(error.message().to_owned()))
    })();
    in_tool.set(false);
    let content = match outcome {
        Ok(content) => content,
        Err(error) => return Err(DispatchError::Report(error)),
    };
    if let Some(schema) = &entry.result
        && let Err(error) = schema.validate(&content)
    {
        return Err(DispatchError::Stop(error::invalid(format!(
            "tool {:?} result {}",
            call.name,
            error.message()
        ))));
    }
    Ok(ToolResult { content })
}

fn service_error(error: ServiceError) -> KoruError {
    let code = match error.kind {
        ServiceErrorKind::Provider => ErrorCode::ProviderFailure,
        ServiceErrorKind::Tool => ErrorCode::ToolFailure,
        ServiceErrorKind::Cancelled => ErrorCode::Cancelled,
    };
    error::coded(code, error.message)
}
