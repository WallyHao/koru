# Koru Bridge Hardening and JSON Schema Enforcement Implementation Plan

**Goal:** Close the remaining evidence gaps of the DESIGN.md `Declaration and conversion` and `Lua/AI bridge` gates before any provider, terminal, or effect capability is added. Specifically: enforce the documented JSON Schema subset on tool declarations, invocation arguments, and declared results; bound service-thread shutdown and document cancellation latency; prove queue bounds and backpressure; add deterministic property tests; and bring the two oversized Lua modules back within the review signal. Everything remains offline and testable with the deterministic fake service; no network, credentials, terminal, process, file, or Git capability is enabled.

**Architecture:** A new Koru-owned `JsonSchema` type compiles a bounded JSON document into an internal schema tree at declaration time and validates values at runtime, all inside the existing single-VM coroutine bridge. The bridge keeps the same protocol (owner resumes the workflow thread, `koru.ai` calls suspend with `yield_with`, bounded channels carry tool calls) but gains an explicit service-completion grace and a cancellation-latency target. Provider adapters are still deferred; the schema and shutdown work is transport-independent.

**Tech Stack:** unchanged: Rust 2024, `mlua = "=0.12.1"` features `["lua55","vendored","async"]`, std threads and `sync_channel`, `sha2`/`thiserror`/`clap`, `tempfile` (dev). No tokio, no Rig, no HTTP client in this increment.

---

## Scope

In scope:

1. The documented v1 JSON Schema subset as a compiled Koru type: objects, properties, required, arrays/items, strings, booleans, finite numbers, safe integers, null, enums, and bounded lengths/ranges; strict rejection of unknown keywords and remote references.
2. Declaration-time compilation of tool `parameters` and optional `result` schemas, replacing the stored raw documents.
3. Runtime argument validation before a tool callback is invoked, and result validation before output is forwarded.
4. Bounded AI-service shutdown: an explicit completion grace, typed timeout when a service does not stop, and a documented cancellation-latency target.
5. Stronger gate evidence: queue-bound/backpressure, greedy-service tool storms, cancellation latency measurement, and deterministic JSON round-trip/property tests.
6. Module-cohesion refactor so no file materially exceeds the recorded review signal.
7. Documentation and status updates.

Out of scope (later increments): real providers and transports, catalog/capability preflight, `config.toml`, credentials, `koru model`/`koru variant`, streaming, retry classification, terminal approval, process/file effects, persistence, `koru shell`, Git.

---

## Specification

The following are normative for this increment. `MUST`/`MUST NOT` are hard requirements; `SHOULD` allows a recorded justification.

### S1. JSON Schema model

- A new module (`src/schema.rs` or `src/json/schema.rs`) exposes a Koru-owned compiled schema. Provider or Lua types MUST NOT appear in it.
- `compile(document: &JsonValue, limits: &JsonLimits) -> Result<JsonSchema>` compiles a bounded document. The document is already subject to the JSON byte/depth/element limits, and `compile` MUST NOT allocate beyond them.
- `JsonSchema::validate(&self, value: &JsonValue) -> Result<()>` returns a bounded, English, path-qualified error on mismatch.
- Supported keywords, and only these:
  - `type`: a string or array of strings from `object`, `array`, `string`, `boolean`, `integer`, `number`, `null`. A JSON integer satisfies `integer` or `number`; a float satisfies only `number`.
  - object: `properties` (name to subschema), `required` (unique strings naming declared properties), `additionalProperties` (boolean only; default `true` to match JSON Schema).
  - array: `items` (one subschema), `minItems`, `maxItems`.
  - string: `minLength`, `maxLength` (UTF-8 byte lengths).
  - number: `minimum`, `maximum` (finite and ordered).
  - integer: `minimum`, `maximum` within `+/- (2^53 - 1)`.
  - any type: `enum` (bounded list of values), `description` (annotation, ignored).
- The tool `parameters` document MUST describe an object at its root; a non-object root fails declaration validation.
- `$ref`, `$schema`, `$id`, `oneOf`, `anyOf`, `allOf`, `not`, `patternProperties`, `propertyNames`, `pattern`, `format`, `default`, `examples`, and every other unknown keyword MUST fail validation with a message naming the keyword and JSON path. This is an explicit documented subset, not full JSON Schema.
- `enum` with an empty list, duplicate entries, or entries that do not match the declared `type` fails validation.

### S2. Declaration-time enforcement

- `ToolDeclaration` stores compiled schemas (`parameters: JsonSchema`, `result: Option<JsonSchema>`) rather than raw documents.
- `CommandDeclaration::new` MUST compile both schemas; a compile failure is a `Validation` error naming the tool and path. This keeps one validator shared by `koru check` and the runtime.
- `ToolSpec.parameters` in the AI contract becomes the compiled schema so adapters can report whether a schema exceeds their supported subset. `ToolSpec` remains provider-neutral.
- `koru check` continues to invoke no callback and performs no effect.

### S3. Runtime enforcement

- Before invoking a tool callback, the bridge MUST validate the model-supplied arguments against the tool's `parameters`. A mismatch MUST NOT invoke the callback and MUST fail the run as a `Validation` error naming the tool and JSON path.
- When a tool declares `result`, the bridge MUST validate the callback's return value before forwarding it to the service. A mismatch MUST fail the run as a `Validation` error and MUST NOT forward partial output.
- Argument and result validation MUST occur inside the existing budget/terminal discipline: validation runs while the workflow is suspended, and a terminal context still stops the run.
- Validation errors MUST be bounded and MUST NOT echo unbounded argument data.

### S4. Service shutdown and cancellation

- The owner MUST NOT block indefinitely on a service thread. After the agent loop ends, the owner drops its channel endpoints and waits for the service to finish with an explicit grace (`AI_SERVICE_JOIN_GRACE`, a hard constant).
- If the service does not finish within the grace, the run MUST return a `Timeout` error, drop the `JoinHandle` (detaching the thread), and report the abandoned service. This is defense-in-depth: v1 services are in-process and MUST terminate when their endpoints close, but a non-cooperative adapter must not hang Koru.
- While the workflow is suspended, cancellation and deadline MUST be observed within `CANCELLATION_LATENCY_TARGET = 50 ms` of the signal. The existing `WAIT_TICK` (5 ms) is the mechanism; this increment documents and tests the target.
- Dropping a future or channel MUST NOT be described as process cleanup. Real subprocess reaping remains a later gate.

### S5. Error and context semantics

- Declaration schema compile failure, argument mismatch, and result mismatch all use `ErrorCode::Validation`. Messages include the tool name and a JSON Pointer path.
- Service grace expiry uses `ErrorCode::Timeout`; a service-reported failure keeps the existing `provider_failure`/`tool_failure` mapping.
- Terminal context states (`Exhausted`, `Cancelled`, `TimedOut`) remain non-resettable by Lua, including through `pcall`.

### S6. Module cohesion

- `src/lua/json.rs` and `src/lua/bridge.rs` currently exceed the recorded review signal. Split each along ownership lines (for example `bridge/` drive loop, tool dispatch, request/result encoding; `json/` install, conversion, emission) so no resulting file materially exceeds roughly 250 nonblank, noncomment lines. If a cohesive split is worse, the module header MUST record why it remains large.

---

## Task 1: JSON Schema core

Files: `src/schema.rs` (or `src/json/schema.rs`), `src/json.rs`, `src/lib.rs`, `tests/schema.rs`.

1. Define `JsonSchema`, its internal node types, `compile`, and `validate`.
2. Implement the supported-keyword set from S1 and strict rejection of everything else, with path-qualified messages.
3. Integrate with `JsonLimits` so schema size, depth, enum length, and message length stay bounded.
4. Tests: a table of accepted documents; rejection of each unsupported keyword; type/enum/required/additionalProperties/bounds matrix; integer safe-range; nested path in error messages; schema depth/size limits.

## Task 2: Declaration and contract integration

Files: `src/declaration.rs`, `src/lua/declaration.rs`, `src/ai/mod.rs`, `src/lua/command.rs`, `tests/declaration.rs`, `tests/lua_controls.rs`.

1. Compile tool `parameters`/`result` at declaration time; store compiled schemas.
2. Require an object root for `parameters`; keep `MAX_TOOLS`, name, and description rules unchanged.
3. Change `ToolSpec.parameters` to the compiled schema; update the fake service and tests.
4. Tests: valid schema, non-object root, unsupported keyword, malformed enum/required, badge of duplicate/missing fields, unchanged `exemptions` rejection.

## Task 3: Runtime enforcement in the bridge

Files: `src/lua/bridge.rs`, `src/lua/command.rs`, `tests/bridge.rs`.

1. Validate tool arguments before dispatch, failing without invoking the callback.
2. Validate declared tool results before forwarding.
3. Keep nested-call rejection, serial order, and budget charging intact.
4. Tests: bad arguments reject before callback (assert the callback never ran); declared result mismatch fails; valid schema round-trips a non-trivial object/array/enum.

## Task 4: Shutdown grace, latency, and gate evidence

Files: `src/lua/bridge.rs`, `src/ai/mod.rs`, `tests/bridge.rs`.

1. Add the completion grace (S4) and map expiry to `Timeout`; detach the abandoned thread.
2. Add a test-only or documented mechanism to exercise the grace without a long test; a short grace constant with a service that blocks longer is acceptable.
3. Add a cancellation-latency test: cancel while the workflow waits and assert the run returns a terminal error within a generous bound around the target.
4. Add a queue-bound test: a service that emits many tool calls still completes with the total bounded by the tool budget, proving backpressure rather than unbounded buffering.

## Task 5: Deterministic property tests

Files: `tests/json_properties.rs`, `tests/schema.rs`.

1. A tiny deterministic generator (fixed seed, in-test) produces values across types, nesting, and boundaries.
2. Assert `to_json`/`from_json` round-trip for accepted values and consistent rejection for rejected ones; no panics.
3. Cover integer/float boundaries, long strings, deep nesting, and cycle construction.

## Task 6: Cohesion refactor

Files: `src/lua/json.rs`, `src/lua/bridge.rs`, `src/lua/mod.rs`.

1. Split both modules along ownership lines; keep public behavior identical.
2. Update module headers to state the split rationale and invariants.

## Task 7: Documentation and status

Files: `docs/lua-api.md`, `docs/declaration.md`, `docs/implementation.md`, `docs/DESIGN.md`, `README.md`.

1. Document the supported JSON Schema subset and the rejection of everything outside it.
2. Document argument/result validation timing, the shutdown grace, and the cancellation-latency target.
3. Update `implementation.md` evidence and remove the now-closed schema/shutdown gaps; update the DESIGN remaining-decision list and the stripped-size measurement.

---

## Gate evidence mapping

| DESIGN gate | This increment's evidence |
| --- | --- |
| Declaration and conversion | schema compile/reject table, declaration integration tests, property tests |
| Lua/AI bridge | argument/result enforcement, queue bounds, serial order still proven, cancellation/deadline observation |
| Cancellation and budgets | cancellation-latency measurement, grace timeout, greedy-service tool-storm bound |

Provider, Git, persistence, and terminal gates remain unaddressed here by design.

## Completion checks

- `just check` passes (fmt, Clippy `-D warnings`, tests, Rustdoc).
- `cargo build --locked --release` and record the stripped size.
- No network, credential, terminal, process, file, or Git code path is added.
- Conventional commits authored `wallyhao <wallyhao@qq.com>`, one logical change each, subject at most 72 characters.

## Risks and fallbacks

- Strictness could reject schemas that live scripts actually use. Mitigation: the subset is the already-documented v1 subset; anything outside it fails loudly with the keyword and path, and tests pin each supported keyword.
- A tight completion grace could misfire on a slow but cooperative adapter. Mitigation: the grace only applies after the loop ends and the endpoints close; it is a large multiple of the wait tick, and the fake service tests both the normal and expiry paths.
- Cancellation-latency assertions are timing-sensitive. Mitigation: use a generous upper bound around the documented target and assert the terminal outcome, not a precise duration.
- Refactoring the two modules can churn the diff. Mitigation: behavior-preserving moves only, with the existing tests as the contract.

## Follow-on phases (sketch, not in this increment)

- **Phase 2C - Provider boundary.** Catalog parsing, capability/adapter descriptors, preflight intersection, `config.toml` selection, environment credentials with redaction, `koru model`/`koru variant` against a fixture transport.
- **Phase 2D - Real transport and `koru shell`.** Pin an HTTP client, wire one implemented protocol, add the terminal approval adapter, execute the prepared shell action under context-bound limits, and enable the end-to-end command.
- **Phase 3 - Git.** Snapshot, plan, grouped publication, recovery, and concurrency behind their dedicated gates.
