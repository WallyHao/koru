# Koru Lua runtime API (version 1)

Status: the `koru.json` conversion and the single-VM `koru.ai` bridge are
implemented and exercised with a deterministic fake service. No real provider is
wired, so `koru <command>` still reports `unsupported_capability`; the runtime
API is reachable through the library and its tests only.

## `koru.json`

Explicit conversion between Lua values and JSON. Conversion happens at the
boundary of the VM; untrusted values are bounded before allocation.

| Lua | JSON | Notes |
| --- | --- | --- |
| `koru.json.null` | null | A bare Lua `nil` is rejected; it is not JSON null. |
| boolean | boolean | |
| integer | integer | Must stay within `+/- (2^53 - 1)`. |
| number | number | Must be finite; `math.huge` is rejected. |
| string | string | Must be valid UTF-8. |
| contiguous `1..n` table | array | Sparse or non-contiguous tables are rejected. |
| string-keyed table | object | Mixed array/object keys are rejected. |
| `koru.json.array(t)` | array | Tags `t` so an empty array is unambiguous. |
| `koru.json.object(t)` | object | Tags `t` so an empty object is unambiguous. |

A plain empty table converts to an empty object; use `koru.json.array({})` for
an empty array. Functions, userdata, cycles, and values over the depth, element,
or byte limits are rejected with a typed error.

## `koru.ai`

- `koru.ai.ask(prompt)` makes one logical tool-free model call.
- `koru.ai.run({ prompt = ..., tools = { ... }, max_turns = ... })` runs a
  bounded agent loop. `tools` is a list of declared tool names; unknown names
  fail before any request. `max_turns` defaults to 8 and is capped at 32.

Both return an `AiResult` table with `text`, `finish_reason`, `model`,
`input_tokens`, `output_tokens`, and `request_id`; unknown usage stays `nil`.
`finish_reason` is one of `stop`, `length`, `tool_calls`, `content_filter`, or
`other`.

Nested AI calls from a tool callback are rejected. AI calls fail with a typed
Lua error on provider or tool failure; the workflow error is categorized as
`provider_failure`, `tool_failure`, `cancelled`, `timeout`, or
`budget_exhausted`.

## Tools

Declared in the command declaration (see `docs/declaration.md`). Each tool has a
`name`, `description`, JSON-schema `parameters`, an optional `result` schema, and
a `run` callback. The callback receives the arguments as a Koru JSON value and
returns a JSON value. Registration is immutable per agent run: `koru.ai.run`
selects a subset by name, and a model cannot add or redefine tools.

## Bridge behavior

- The workflow runs in a Lua coroutine owned by one VM thread. `koru.ai` calls
  suspend with `yield_with`; the owner resumes the workflow with the result.
- While suspended, the owner services tool calls serially in stable order and
  does not hold a VM access guard across the wait.
- A service runs on its own thread and communicates over bounded channels. The
  owner checks cancellation and the deadline on every wait tick and stops the run
  without resuming the workflow.
- Model-request and tool-call budgets are charged from the shared
  `ExecutionContext`; turns are enforced by the service and the request cap.

## Not implemented

Real providers and credentials, JSON-schema and argument-value validation,
streaming, retry classification, and terminal/process effects. See
`docs/implementation.md` for details.
