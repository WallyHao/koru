//! Drive one workflow coroutine and bridge its AI calls to a service.
//!
//! The VM owner resumes the workflow thread; each `koru.ai` call suspends with
//! `yield_with` and returns a request to the owner. The owner then runs the
//! agent loop over bounded channels, dispatching registered tool callbacks
//! serially while the workflow is suspended, and resumes it with the result.
use super::{
    command::{LoadedCommand, ToolEntry},
    error, json,
};
use crate::{
    ai::{
        AiRequest, AiResult, AiService, ChannelEvents, Event, FinishReason, Reply, ServiceError,
        ServiceErrorKind, ToolCall, ToolResult, ToolSpec,
    },
    error::{ErrorCode, KoruError, Result},
    json::{JsonLimits, JsonValue},
    runtime::{ExecutionContext, Resources},
};
use mlua::{Lua, Table, Value, thread::ThreadStatus};
use std::{
    cell::Cell,
    collections::BTreeMap,
    rc::Rc,
    sync::{
        Arc, Mutex,
        mpsc::{RecvTimeoutError, sync_channel},
    },
    time::{Duration, Instant},
};

const CHANNEL_CAPACITY: usize = 8;
const WAIT_TICK: Duration = Duration::from_millis(5);
const DEFAULT_MAX_TURNS: u32 = 8;
const MAX_MAX_TURNS: u32 = 32;
const MAX_PROMPT_BYTES: usize = 256 * 1024;

/// Execute one loaded workflow, bridging AI calls to `service`.
pub(super) fn run(
    loaded: LoadedCommand,
    service: Box<dyn AiService>,
    args: &JsonValue,
) -> Result<JsonValue> {
    let LoadedCommand {
        declaration: _,
        run,
        tools,
        sandbox,
    } = loaded;
    let context = sandbox.context().clone();
    let lua = sandbox.into_lua();
    let json_table = json::install(&lua)?;
    let in_tool = Rc::new(Cell::new(false));
    let specs: Rc<BTreeMap<String, ToolSpec>> = Rc::new(
        tools
            .iter()
            .map(|(name, entry)| (name.clone(), entry.spec.clone()))
            .collect(),
    );
    let ai_table = build_ai(&lua, specs, Rc::clone(&in_tool))?;
    let koru = lua
        .create_table()
        .map_err(|error| error::invalid(format!("cannot build koru: {error}")))?;
    koru.set("json", json_table)
        .map_err(|error| error::invalid(format!("cannot build koru: {error}")))?;
    koru.set("ai", ai_table)
        .map_err(|error| error::invalid(format!("cannot build koru: {error}")))?;
    let argument = json::from_json(&lua, args)?;
    let workflow = lua
        .create_thread(run)
        .map_err(|error| error::invalid(format!("cannot start the workflow: {error}")))?;
    let service = Arc::new(Mutex::new(service));
    let mut value = workflow
        .resume::<Value>((koru, argument))
        .map_err(|error| error::map(&context, error, "workflow failed"))?;
    while workflow.status() == ThreadStatus::Resumable {
        let request = decode_request(&value)?;
        let result = drive_agent(&service, request, &context, &tools, &lua, &in_tool)?;
        let encoded = encode_result(&lua, &result)
            .map_err(|error| error::invalid(format!("cannot return the AI result: {error}")))?;
        value = workflow
            .resume::<Value>(encoded)
            .map_err(|error| error::map(&context, error, "workflow failed"))?;
    }
    if value.is_nil() {
        Ok(JsonValue::Null)
    } else {
        json::to_json(&value, &JsonLimits::default())
    }
}

fn build_ai(
    lua: &Lua,
    specs: Rc<BTreeMap<String, ToolSpec>>,
    in_tool: Rc<Cell<bool>>,
) -> Result<Table> {
    let ai = lua
        .create_table()
        .map_err(|error| error::invalid(format!("cannot build koru.ai: {error}")))?;
    let ask_flag = Rc::clone(&in_tool);
    let ask = lua
        .create_async_function(move |lua, prompt: String| {
            let flag = Rc::clone(&ask_flag);
            async move {
                let request = AiRequest {
                    prompt,
                    tools: Vec::new(),
                    max_turns: 1,
                };
                ai_call(lua, flag, request).await
            }
        })
        .map_err(|error| error::invalid(format!("cannot build koru.ai.ask: {error}")))?;
    ai.set("ask", ask)
        .map_err(|error| error::invalid(format!("cannot build koru.ai: {error}")))?;
    let run_flag = Rc::clone(&in_tool);
    let run_fn = lua
        .create_async_function(move |lua, options: Table| {
            let specs = Rc::clone(&specs);
            let flag = Rc::clone(&run_flag);
            async move {
                let request = read_run_options(&options, &specs)?;
                ai_call(lua, flag, request).await
            }
        })
        .map_err(|error| error::invalid(format!("cannot build koru.ai.run: {error}")))?;
    ai.set("run", run_fn)
        .map_err(|error| error::invalid(format!("cannot build koru.ai: {error}")))?;
    Ok(ai)
}

async fn ai_call(lua: Lua, flag: Rc<Cell<bool>>, request: AiRequest) -> mlua::Result<Value> {
    if flag.get() {
        return Err(mlua::Error::RuntimeError(
            "nested AI calls from a tool callback are not supported".to_owned(),
        ));
    }
    let encoded = encode_request(&lua, &request)?;
    let reply: Value = lua.yield_with(encoded).await?;
    Ok(reply)
}

fn read_run_options(
    options: &Table,
    specs: &BTreeMap<String, ToolSpec>,
) -> mlua::Result<AiRequest> {
    let mut prompt = None;
    let mut tool_names = Vec::new();
    let mut max_turns = DEFAULT_MAX_TURNS;
    for pair in options.pairs::<String, Value>() {
        let (key, value) = pair?;
        match key.as_str() {
            "prompt" => {
                let text = value
                    .as_string()
                    .ok_or_else(|| mlua::Error::RuntimeError("`prompt` must be a string".into()))?;
                let text = text.to_str().map_err(|_| {
                    mlua::Error::RuntimeError("`prompt` must be valid UTF-8".into())
                })?;
                if text.len() > MAX_PROMPT_BYTES {
                    return Err(mlua::Error::RuntimeError("`prompt` is too long".into()));
                }
                prompt = Some(text.as_ref().to_owned());
            }
            "tools" => {
                let list = value
                    .as_table()
                    .ok_or_else(|| mlua::Error::RuntimeError("`tools` must be an array".into()))?;
                for item in list.sequence_values::<String>() {
                    let name = item?;
                    if !specs.contains_key(&name) {
                        return Err(mlua::Error::RuntimeError(format!("unknown tool {name:?}")));
                    }
                    tool_names.push(name);
                }
            }
            "max_turns" => {
                let turns = value.as_integer().ok_or_else(|| {
                    mlua::Error::RuntimeError("`max_turns` must be an integer".into())
                })?;
                if !(1..=i64::from(MAX_MAX_TURNS)).contains(&turns) {
                    return Err(mlua::Error::RuntimeError(
                        "`max_turns` is out of range".into(),
                    ));
                }
                max_turns = turns as u32;
            }
            other => {
                return Err(mlua::Error::RuntimeError(format!(
                    "unknown ai.run option {other:?}"
                )));
            }
        }
    }
    let prompt =
        prompt.ok_or_else(|| mlua::Error::RuntimeError("ai.run needs a `prompt`".into()))?;
    let mut tools = Vec::with_capacity(tool_names.len());
    for name in tool_names {
        if let Some(spec) = specs.get(&name) {
            tools.push(spec.clone());
        }
    }
    Ok(AiRequest {
        prompt,
        tools,
        max_turns,
    })
}

fn drive_agent(
    service: &Arc<Mutex<Box<dyn AiService>>>,
    request: AiRequest,
    context: &ExecutionContext,
    tools: &BTreeMap<String, ToolEntry>,
    lua: &Lua,
    in_tool: &Rc<Cell<bool>>,
) -> Result<AiResult> {
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
                let reply = dispatch_tool(tools, lua, in_tool, &call);
                if reply_tx.send(Reply::Result(reply)).is_err() {
                    break;
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
    let _ = worker.join();
    if let Some(error) = terminal {
        return Err(error);
    }
    context.ensure_active(Instant::now())?;
    outcome.map_err(service_error)
}

fn dispatch_tool(
    tools: &BTreeMap<String, ToolEntry>,
    lua: &Lua,
    in_tool: &Rc<Cell<bool>>,
    call: &ToolCall,
) -> std::result::Result<ToolResult, ServiceError> {
    let Some(entry) = tools.get(&call.name) else {
        return Err(ServiceError::tool(format!("unknown tool {:?}", call.name)));
    };
    in_tool.set(true);
    let outcome = (|| -> std::result::Result<ToolResult, ServiceError> {
        let argument = json::from_json(lua, &call.arguments)
            .map_err(|error| ServiceError::tool(error.message().to_owned()))?;
        let value = entry
            .callback
            .call::<Value>(argument)
            .map_err(|error| ServiceError::tool(error.to_string()))?;
        let content = json::to_json(&value, &JsonLimits::default())
            .map_err(|error| ServiceError::tool(error.message().to_owned()))?;
        Ok(ToolResult { content })
    })();
    in_tool.set(false);
    outcome
}

fn service_error(error: ServiceError) -> KoruError {
    let code = match error.kind {
        ServiceErrorKind::Provider => ErrorCode::ProviderFailure,
        ServiceErrorKind::Tool => ErrorCode::ToolFailure,
        ServiceErrorKind::Cancelled => ErrorCode::Cancelled,
    };
    error::coded(code, error.message)
}

fn encode_request(lua: &Lua, request: &AiRequest) -> mlua::Result<Value> {
    let table = lua.create_table()?;
    table.set("koru_kind", "ai_request")?;
    table.set("prompt", request.prompt.as_str())?;
    table.set("max_turns", request.max_turns)?;
    let tools = lua.create_table()?;
    for (index, spec) in request.tools.iter().enumerate() {
        let entry = lua.create_table()?;
        entry.set("name", spec.name.as_str())?;
        entry.set("description", spec.description.as_str())?;
        entry.set("parameters", from_json_mlua(lua, &spec.parameters)?)?;
        tools.set(index + 1, entry)?;
    }
    table.set("tools", tools)?;
    Ok(Value::Table(table))
}

fn decode_request(value: &Value) -> Result<AiRequest> {
    let table = value
        .as_table()
        .ok_or_else(|| error::invalid("invalid AI request"))?;
    if read_optional_string(table, "koru_kind")?.as_deref() != Some("ai_request") {
        return Err(error::invalid("invalid AI request"));
    }
    let prompt = read_required_string(table, "prompt")?;
    let max_turns = table
        .get::<u32>("max_turns")
        .map_err(|_| error::invalid("AI request `max_turns` must be an integer"))?;
    let list = table
        .get::<Table>("tools")
        .map_err(|_| error::invalid("AI request `tools` must be an array"))?;
    let mut tools = Vec::new();
    for entry in list.sequence_values::<Table>() {
        let entry = entry.map_err(|_| error::invalid("AI request tool must be a table"))?;
        let name = read_required_string(&entry, "name")?;
        let description = read_required_string(&entry, "description")?;
        let parameters = entry
            .get::<Value>("parameters")
            .map_err(|_| error::invalid("AI request tool parameters are missing"))?;
        tools.push(ToolSpec {
            name,
            description,
            parameters: json::to_json(&parameters, &JsonLimits::default())?,
        });
    }
    Ok(AiRequest {
        prompt,
        tools,
        max_turns,
    })
}

fn encode_result(lua: &Lua, result: &AiResult) -> mlua::Result<Value> {
    let table = lua.create_table()?;
    table.set("koru_kind", "ai_result")?;
    table.set("text", result.text.as_str())?;
    table.set("finish_reason", finish_reason_name(&result.finish_reason))?;
    table.set("model", result.model.as_str())?;
    table.set("input_tokens", result.usage.input_tokens)?;
    table.set("output_tokens", result.usage.output_tokens)?;
    table.set("request_id", result.request_id.as_deref())?;
    Ok(Value::Table(table))
}

fn from_json_mlua(lua: &Lua, value: &JsonValue) -> mlua::Result<Value> {
    json::from_json(lua, value)
        .map_err(|error| mlua::Error::RuntimeError(error.message().to_owned()))
}

fn read_required_string(table: &Table, key: &str) -> Result<String> {
    table
        .get::<String>(key)
        .map_err(|_| error::invalid(format!("AI message field `{key}` must be a string")))
}

fn read_optional_string(table: &Table, key: &str) -> Result<Option<String>> {
    table
        .get::<Option<String>>(key)
        .map_err(|_| error::invalid(format!("AI message field `{key}` must be a string")))
}

fn finish_reason_name(reason: &FinishReason) -> &str {
    match reason {
        FinishReason::Stop => "stop",
        FinishReason::Length => "length",
        FinishReason::ToolCalls => "tool_calls",
        FinishReason::ContentFilter => "content_filter",
        FinishReason::Other(_) => "other",
    }
}
