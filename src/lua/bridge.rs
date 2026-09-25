//! Drive one workflow coroutine and bridge its AI calls to a service.
//!
//! The VM owner resumes the workflow thread; each `koru.ai` call stores its
//! request in a shared slot, suspends with `yield_with`, and returns what the
//! owner resumes it with. The owner then runs the agent loop over bounded
//! channels, validating tool arguments and results against compiled schemas and
//! dispatching registered callbacks serially while the workflow is suspended.
//! The drive loop, option parsing, and result encoding live in submodules so
//! each keeps one responsibility.
use super::{
    command::{ApprovalProvider, LoadedCommand},
    error, json,
};
use crate::{
    ai::{AiRequest, AiService, ToolSpec},
    error::Result,
    json::{JsonLimits, JsonValue},
};
use mlua::{Lua, Table, Value, thread::ThreadStatus};
use std::{
    cell::{Cell, RefCell},
    collections::BTreeMap,
    rc::Rc,
    time::Duration,
};

mod drive;
mod effects;
mod encode;
mod options;

use drive::drive_agent;
use effects::{ShellRequest, build_shell, complete_shell};
use encode::encode_result;
use options::{JsonRequest, read_json_options, read_run_options};

/// Bounded queue capacity for the owner/service channels.
const CHANNEL_CAPACITY: usize = 8;
/// How often the owner rechecks cancellation and the deadline while waiting.
const WAIT_TICK: Duration = Duration::from_millis(5);

/// Time the owner waits for a service thread to stop before detaching it.
///
/// A terminal run state takes precedence over this bound, so cancellation and
/// deadlines are reported even when a service ignores channel closure.
pub const AI_SERVICE_JOIN_GRACE: Duration = Duration::from_millis(250);
/// Target from observing a cancellation signal to returning while suspended.
pub const CANCELLATION_LATENCY_TARGET: Duration = Duration::from_millis(50);

/// The AI request a suspended workflow call left for the owner to service.
enum PendingRequest {
    Ai(AiRequest),
    Json(Box<JsonRequest>),
    Shell(ShellRequest),
}

type Pending = Rc<RefCell<Option<PendingRequest>>>;

/// Execute one loaded workflow, bridging AI calls to `service`.
pub(super) fn run(
    loaded: LoadedCommand,
    service: Box<dyn AiService>,
    args: &JsonValue,
    approval: &mut dyn ApprovalProvider,
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
    let pending: Pending = Rc::new(RefCell::new(None));
    let specs: Rc<BTreeMap<String, ToolSpec>> = Rc::new(
        tools
            .iter()
            .map(|(name, entry)| (name.clone(), entry.spec.clone()))
            .collect(),
    );
    let ai_table = build_ai(&lua, specs, Rc::clone(&in_tool), Rc::clone(&pending))?;
    let shell_table = build_shell(&lua, Rc::clone(&in_tool), Rc::clone(&pending))?;
    let koru = lua
        .create_table()
        .map_err(|error| error::invalid(format!("cannot build koru: {error}")))?;
    koru.set("json", json_table)
        .map_err(|error| error::invalid(format!("cannot build koru: {error}")))?;
    koru.set("ai", ai_table)
        .map_err(|error| error::invalid(format!("cannot build koru: {error}")))?;
    koru.set("shell", shell_table)
        .map_err(|error| error::invalid(format!("cannot build koru: {error}")))?;
    let argument = json::from_json(&lua, args)?;
    let workflow = lua
        .create_thread(run)
        .map_err(|error| error::invalid(format!("cannot start the workflow: {error}")))?;
    let service = std::sync::Arc::new(std::sync::Mutex::new(service));
    let mut value = workflow
        .resume::<Value>((koru, argument))
        .map_err(|error| error::map(&context, error, "workflow failed"))?;
    while workflow.status() == ThreadStatus::Resumable {
        let pending_request = pending
            .borrow_mut()
            .take()
            .ok_or_else(|| error::invalid("workflow suspended without an AI request"))?;
        let encoded = match pending_request {
            PendingRequest::Ai(request) => {
                let result = drive_agent(&service, request, &context, &tools, &lua, &in_tool)?;
                reserve_ai_result(&context, &result)?;
                encode_result(&lua, &result).map_err(|error| {
                    error::invalid(format!("cannot return the AI result: {error}"))
                })?
            }
            PendingRequest::Json(request) => {
                let result =
                    drive_agent(&service, request.request, &context, &tools, &lua, &in_tool)?;
                reserve_ai_result(&context, &result)?;
                encode_json_result(&lua, &result, &request.schema)?
            }
            PendingRequest::Shell(request) => complete_shell(&lua, request, &context, approval)
                .map_err(|cause| error::invalid(format!("cannot return shell result: {cause}")))?,
        };
        value = workflow
            .resume::<Value>(encoded)
            .map_err(|error| error::map(&context, error, "workflow failed"))?;
    }
    context.ensure_active(std::time::Instant::now())?;
    if value.is_nil() {
        Ok(JsonValue::Null)
    } else {
        json::to_json(&value, &JsonLimits::default())
    }
}

fn reserve_ai_result(
    context: &crate::runtime::ExecutionContext,
    result: &crate::ai::AiResult,
) -> Result<()> {
    context.reserve(
        crate::runtime::Resources {
            bytes: result.text.len() as u64,
            ..crate::runtime::Resources::ZERO
        },
        std::time::Instant::now(),
    )
}

fn build_ai(
    lua: &Lua,
    specs: Rc<BTreeMap<String, ToolSpec>>,
    in_tool: Rc<Cell<bool>>,
    pending: Pending,
) -> Result<Table> {
    let ai = lua
        .create_table()
        .map_err(|error| error::invalid(format!("cannot build koru.ai: {error}")))?;
    let ask_flag = Rc::clone(&in_tool);
    let ask_pending = Rc::clone(&pending);
    let ask = lua
        .create_async_function(move |lua, prompt: String| {
            let flag = Rc::clone(&ask_flag);
            let slot = Rc::clone(&ask_pending);
            async move {
                let request = AiRequest {
                    prompt,
                    tools: Vec::new(),
                    max_turns: 1,
                };
                ai_call(lua, flag, slot, PendingRequest::Ai(request)).await
            }
        })
        .map_err(|error| error::invalid(format!("cannot build koru.ai.ask: {error}")))?;
    ai.set("ask", ask)
        .map_err(|error| error::invalid(format!("cannot build koru.ai: {error}")))?;
    let run_flag = Rc::clone(&in_tool);
    let run_pending = Rc::clone(&pending);
    let run_fn = lua
        .create_async_function(move |lua, options: Table| {
            let specs = Rc::clone(&specs);
            let flag = Rc::clone(&run_flag);
            let slot = Rc::clone(&run_pending);
            async move {
                let request = read_run_options(&options, &specs)?;
                ai_call(lua, flag, slot, PendingRequest::Ai(request)).await
            }
        })
        .map_err(|error| error::invalid(format!("cannot build koru.ai.run: {error}")))?;
    ai.set("run", run_fn)
        .map_err(|error| error::invalid(format!("cannot build koru.ai: {error}")))?;
    let json_flag = Rc::clone(&in_tool);
    let json_pending = Rc::clone(&pending);
    let ask_json = lua
        .create_async_function(move |lua, options: Table| {
            let flag = Rc::clone(&json_flag);
            let slot = Rc::clone(&json_pending);
            async move {
                let request = read_json_options(&options)?;
                ai_call(lua, flag, slot, PendingRequest::Json(Box::new(request))).await
            }
        })
        .map_err(|error| error::invalid(format!("cannot build koru.ai.ask_json: {error}")))?;
    ai.set("ask_json", ask_json)
        .map_err(|error| error::invalid(format!("cannot build koru.ai: {error}")))?;
    Ok(ai)
}

async fn ai_call(
    lua: Lua,
    flag: Rc<Cell<bool>>,
    pending: Pending,
    request: PendingRequest,
) -> mlua::Result<Value> {
    if flag.get() {
        return Err(mlua::Error::RuntimeError(
            "nested AI calls from a tool callback are not supported".to_owned(),
        ));
    }
    *pending.borrow_mut() = Some(request);
    lua.yield_with(Value::Nil).await
}

fn encode_json_result(
    lua: &Lua,
    result: &crate::ai::AiResult,
    schema: &crate::schema::JsonSchema,
) -> Result<Value> {
    const MAX_STRUCTURED_RESPONSE_BYTES: usize = 256 * 1024;
    if result.text.is_empty() {
        return Err(error::invalid("structured model response is empty"));
    }
    if result.text.len() > MAX_STRUCTURED_RESPONSE_BYTES {
        return Err(crate::error::KoruError::new(
            crate::error::ErrorCode::BudgetExhausted,
            "structured model response exceeds the byte limit",
        ));
    }
    let data =
        crate::json::parse(result.text.as_bytes(), &JsonLimits::default()).map_err(|parse| {
            error::invalid(format!("structured model response: {}", parse.message()))
        })?;
    schema.validate(&data).map_err(|validation| {
        error::invalid(format!(
            "structured model response: {}",
            validation.message()
        ))
    })?;
    json::from_json(lua, &data)
}
