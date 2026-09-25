# Koru Lua runtime API (version 1)

Status: `koru.json`, `koru.ai`, the approved Linux shell API, and the Linux staged Git
snapshot/planning API are available through `koru <command>` with the selected
provider. The bridge is tested with deterministic fake services and recorded
provider transports.

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

`koru.ai.ask_json({ prompt = ..., schema = ..., mode = "prompt_validate" })`
makes one tool-free request using an explicit prompt-and-validate fallback. Koru
adds the bounded schema to the prompt, parses exactly one JSON value from the
answer, validates it with the documented schema subset, and returns only the
validated Lua value. This mode does not claim provider-native structured output.
Malformed JSON, empty/refusal text, schema mismatch, and unsupported schema
keywords fail with `validation`; an oversized response fails with
`budget_exhausted`.

`koru.shell.script(text, { cwd = ..., explanation = ... })` runs the exact
script through one terminal approval and returns `ProcessResult` or `nil, error`.
The cwd must name an existing directory. `stdout` and `stderr` are bounded UTF-8
text; invalid byte sequences are replaced when the process result enters Lua.
`ProcessResult` contains
`exit_code` or `signal`, `stdout`, `stderr`, `stdout_truncated`, and
`stderr_truncated`. Model-proposed scripts cannot add child environment values.

`koru.git.snapshot()` requests one approval for a fixed, read-only
`git diff --cached` action. On approval it returns bounded staged change
summaries, opaque change IDs, and bounded diff excerpts; file paths remain on
the Rust side. The API is Linux-only and returns `nil, error` if approval is
denied or the staged diff cannot be read.

`koru.git.validate(plan)` accepts a model proposal against the current
in-memory snapshot. It requires every snapshot change ID exactly once and
validates each Conventional Commit subject and rationale before producing a
plan ID. The snapshot and plan IDs are tied to the staged diff and validated
grouping. The `commit` example prints this plan for review. It does not write
the Git index, worktree, refs, or commit objects.

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

### Supported schema subset

`parameters` and `result` are compiled once to a Koru-owned schema. Only this
subset is accepted; any other keyword fails declaration validation with the
keyword and JSON path named.

- `type`: a string or array of strings from `object`, `array`, `string`,
  `boolean`, `integer`, `number`, `null`. A JSON integer satisfies `integer` or
  `number`.
- object: `properties`, `required`, `additionalProperties` (boolean, default
  `true`).
- array: `items`, `minItems`, `maxItems`.
- string: `minLength`, `maxLength` (UTF-8 bytes).
- number/integer: `minimum`, `maximum` (finite and ordered; integer bounds must
  be whole numbers in the safe range).
- any type: `enum` (unique, non-empty, matching the declared type) and the
  `description` annotation.

`parameters` must describe an object at its root. `$ref`, `$schema`, `$id`,
`oneOf`, `anyOf`, `allOf`, `not`, `patternProperties`, `propertyNames`,
`pattern`, `format`, `default`, `examples`, and every other unknown keyword are
rejected rather than ignored.

### Validation timing

The owner validates model-supplied arguments against `parameters` before the
callback runs; a mismatch fails the run as a `validation` error and the callback
is never invoked. When `result` is declared, the callback's return value is
validated before it is forwarded, and a mismatch fails the run as a `validation`
error. Errors name the tool and a JSON Pointer path.

## Bridge behavior

- The workflow runs in a Lua coroutine owned by one VM thread. `koru.ai` calls
  suspend with `yield_with`; the owner resumes the workflow with the result.
- While suspended, the owner services tool calls serially in stable order and
  does not hold a VM access guard across the wait.
- A service runs on its own thread and communicates over bounded channels. The
  owner checks cancellation and the deadline on every wait tick and stops the run
  without resuming the workflow.
- A terminal state (cancelled, timed out, budget exhausted, or a validation
  failure) returns immediately without waiting for the service thread; the
  service is detached and the process reaps it when it returns. A normally
  completed service is reaped within `AI_SERVICE_JOIN_GRACE` (250 ms); a service
  that neither completes nor returns within that grace is reported as a
  `timeout`. The target from a cancellation signal to returning is 50 ms.
- Model-request and tool-call budgets are charged from the shared
  `ExecutionContext`; turns are enforced by the service and the request cap.
  Tool dispatch is bounded by the tool-call budget, not just by queue capacity.

## Not implemented

Streaming and Lua file effects. Shell execution is supported on Linux only.
`koru.ai.ask_json` uses prompt-and-validate; providers do not claim native
structured-output capability. See `docs/implementation.md` for details.
