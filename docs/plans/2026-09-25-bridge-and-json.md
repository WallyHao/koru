# Koru Single-VM Bridge and JSON Conversion Implementation Plan

**Goal:** Deliver the `Lua/AI bridge` gate of DESIGN.md: typed Lua-to-JSON conversion, Koru-owned AI/tool contracts, and a single-VM cooperative bridge where a workflow coroutine suspends on an AI call, the VM owner services serial tool callbacks, and bounded channels connect to a provider service. Prove everything with a deterministic fake AI service; no network, credentials, terminal, process, or Git capability is enabled.

**Architecture:** The Lua VM stays on one owner thread. A workflow runs inside a `Thread` (coroutine). `koru.ai.ask` and `koru.ai.run` are mlua async functions that suspend with `yield_with`; the owner resumes the workflow with the final `AiResult`. While suspended, the owner drives the agent loop over bounded `std::sync::mpsc::sync_channel` endpoints to a service thread, invoking registered Lua tool callbacks serially. No executor runtime is required: mlua's `async` feature and synchronous `Thread::resume` drive the futures, proven by `tests/bridge_prototype.rs`.

**Tech Stack:** Rust 2024, `mlua = "=0.12.1"` features `["lua55", "vendored", "async"]` (adds `futures-util`), std threads and `sync_channel`, existing `sha2`/`thiserror`/`clap`. No tokio and no Rig in this increment.

---

## Scope

In scope:

1. Koru-owned JSON value model and Lua conversion: `koru.json.null`, tagged `koru.json.array`/`object`, string keys, contiguous arrays, safe integers, depth/element/byte limits, and rejection of mixed-key tables, cycles, non-finite numbers, functions, invalid UTF-8, and unsupported userdata.
2. AI contracts: `AiRequest`, `AiResult`, `FinishReason`, `Usage`, `ToolSpec`, `ToolCall`, `ToolResult`, `ServiceError`, and the `AiService`/`ServiceEvents` service boundary.
3. Tool declarations in the API version 1 schema: unique name, description, bounded `parameters`, and a required `run` callback; registration is immutable per agent run.
4. The bridge: workflow in a coroutine, `koru.ai.ask`/`koru.ai.run`, serial tool dispatch in stable order, nested AI-call rejection, bounded request/event/reply channels, per-call turn and tool budgets, and cancellation/deadline observed while waiting.
5. A deterministic fake service (`src/ai/fake.rs`) plus tests for every gate bullet.

Out of scope (later increments): real providers, credentials, catalog/capability checks, JSON Schema subset and argument value validation, streaming, retry classification, terminal approval, process/file execution, persistence, Git.

---

## Frozen decisions

- The VM owner drives async functions with `Thread::resume`; no async runtime is added. `yield_with` carries a request table out and a result table back.
- Agent-run protocol over bounded channels (capacity is a hard constant): owner sends `AiRequest`; the service thread sends `ToolCall` events and waits for a `ToolResult` reply per call; the service ends with `Finished(AiResult)` or `Failed(ServiceError)`. The owner polls the execution context on a short receive timeout so cancellation and deadline stop the wait.
- `koru.ai.run` rejects nested calls: the owner sets a per-VM "dispatching tool" flag before invoking a callback; an AI call while the flag is set fails with `unsupported_capability` semantics.
- Tool arguments and results cross the Lua boundary as Koru JSON values, not provider-specific shapes.
- `tools` in `koru.ai.run` is a list of declared tool names; unknown names fail before any service request.

---

## Task 1: JSON model and Lua conversion

Files: `src/json.rs`, `src/lua/json.rs`, `src/lib.rs`, `tests/json.rs`.

1. Define `JsonValue`, `JsonLimits`, and a checked conversion constructor with depth, element-count, string/key byte, and safe-integer rules.
2. Install `koru.json` in the execution environment: `null`, `array`, `object`, and internal convert/emit helpers; the sentinel and tags must not be forgeable from Lua.
3. Reject cycles by tracking visited tables, reject mixed keys, non-finite numbers, functions, invalid UTF-8, and non-sentinel userdata; keep errors bounded and escaped.
4. Round-trip tests: null, booleans, safe/oversized integers, numbers, strings, nested arrays/objects, empty array vs empty object, depth/element/byte limits, cycles, mixed keys, invalid UTF-8.

## Task 2: AI and tool contracts

Files: `src/ai/mod.rs`, `src/ai/fake.rs`, `src/lib.rs`, `tests/ai.rs`.

1. Define the owned request/result/tool types and the `AiService`/`ServiceEvents` trait boundary; no provider or Rig type appears in the contracts.
2. Define the bounded channel protocol and a helper the owner uses to run one agent request against a service thread.
3. Implement `FakeAiService` as deterministic test support: scripted tool-call rounds followed by a final answer, with optional cooperative cancellation checks.
4. Tests: request/result conversion, scripted serial tool calls, service failure propagation, and channel-capacity behavior.

## Task 3: Tool declarations and registration

Files: `src/declaration.rs`, `src/lua/declaration.rs`, `src/lua/command.rs`, `tests/declaration.rs`, `tests/lua_controls.rs`.

1. Accept `tools` in the declaration: array of entries with `name`, `description`, bounded `parameters`, and a `run` function; reject unknown fields, duplicate or malformed names, and oversized documents.
2. Keep `exemptions` rejected until its store exists. Tool metadata is validated but callbacks are not invoked during `check`.
3. Retain per-tool registry keys in `LoadedCommand` for later dispatch and expose the validated `ToolSpec` list.
4. Tests: valid tools, duplicate/invalid names, missing callback, oversized parameters, and unchanged rejection of `exemptions`.

## Task 4: Single-VM bridge

Files: `src/lua/bridge.rs`, `src/lua/command.rs`, `src/lua/mod.rs`, `tests/bridge.rs`.

1. `Workflow::run(loaded, context, service)` installs `koru.ai` and `koru.json`, creates a thread from `run`, and executes the drive loop.
2. `koru.ai.ask(prompt)` and `koru.ai.run{...}` convert options to `AiRequest`, `yield_with` it, and return the resumed `AiResult` (or raise a typed Lua error).
3. The owner loop: on a yielded request, reserve budgets, send to the service, process events serially, dispatch tools through the registry while the "dispatching tool" flag is set, reply with `ToolResult`, and finally resume with `AiResult`.
4. Cancellation and deadline are checked on each receive timeout and before every dispatch; a terminal context stops the run, closes channels, and returns the terminal error without resuming the workflow.
5. Nested AI calls fail explicitly; unknown tool selections fail before any request.
6. Tests: suspended workflow receives serial tool calls in order; multiple AI calls in one workflow; nested call rejected; cancellation while waiting; tool/model/turn budget exhaustion; service failure surfaces as a typed error; unknown tool name rejected.

## Task 5: Documentation and status

Files: `README.md`, `docs/declaration.md`, `docs/implementation.md`, `docs/DESIGN.md`.

1. Document `koru.json` semantics, tool declarations, `koru.ai` behavior, the channel protocol, and the explicit limitations (no real provider, no schema validation yet).
2. Update implementation status, measured evidence, and the remaining decision list; remove the now-resolved empty-tag and yield questions.

## Completion checks

- `just check` passes.
- `cargo build --locked --release` and record the stripped size.
- Confirm no provider, credential, terminal, process, or Git code path exists in the increment.
- Conventional commits authored `wallyhao <wallyhao@qq.com>`, one logical change each.

## Risks and fallbacks

- mlua async yield across nested calls is the main risk and is already prototyped; if a later construct cannot suspend, the fallback is an isolated worker process, recorded before claiming the bound.
- Cycle detection on large tables must stay within the element budget; check limits before deep recursion.
- A misbehaving service thread must never block the owner indefinitely; every wait uses a bounded timeout with a terminal check.
