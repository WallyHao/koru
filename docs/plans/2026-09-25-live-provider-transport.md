# Koru Live Provider Transport and Provider Hardening Plan

**Goal:** Complete the DESIGN.md `Providers` gate on top of the 2C boundary: a
bounded JSON codec, a blocking HTTP transport, one real protocol end to end
(DeepSeek Chat Completions), OpenCode Zen/Go adapters with session continuity,
live catalog fetch with a durable cache, retry classification inside the original
budget, and the provider hardening deferred from 2B/2C. Terminal approval,
process effects, and `koru shell` are Phase 2E; Git is Phase 3.

**Architecture:** A blocking `Transport` trait sits below the adapters and has a
`ureq` (rustls) implementation plus a deterministic fixture. Protocol payloads are
built and parsed by pure functions over a new bounded `JsonValue` JSON codec, so
adapters are unit-tested without network. A `DeepSeekAdapter` and an
`OpenCodeAdapter` implement both `AiService` and `CatalogSource` over the
transport. Catalogs cache as versioned JSON under `$XDG_CACHE_HOME/koru`, written
atomically under a shared lock. Retries are classified and charged to the existing
`ExecutionContext`; a run never replays after a tool may have executed.

**Tech Stack:** unchanged Rust 2024 and crates, plus one new pinned dependency for
blocking TLS HTTP (`ureq` with `rustls`/webpki roots), selected by an explicit
availability spike. No async runtime, no serde, no HTTP code in `src/lua`.

---

## Scope

In scope:

1. A bounded JSON parser and canonical emitter for `JsonValue`, with explicit
   depth/element/byte limits and round-trip tests.
2. A `Transport` trait, its deterministic fixture, and the `ureq` implementation
   with timeouts, size caps, bounded redirects, and a Koru user agent.
3. A Chat Completions protocol layer: request builder and response parser with
   normalized finish reasons, usage, request ids, and tool calls.
4. A DeepSeek adapter implementing `AiService` and `CatalogSource`.
5. OpenCode Zen and Go adapters with separate identities, documented paths, and a
   stable `x-opencode-session` per conversation including retries and tool loops.
6. Live catalog fetch, versioned cache with atomic replacement and a shared lock,
   per-service refresh that preserves previous data on failure, and reporting of a
   removed selected model without silent substitution.
7. Retry classification for transient provider failures, charged to the request
   budget and deadline, never replaying after tool execution.
8. Secret redaction at transport and adapter boundaries; credentials only in
   headers; no prompt, diff, or tool-output logging by default.
9. Hardening: a shared `FileLock` with stale-lock detection used by config and
   cache, catalog-cache durability, and a size/perf review.
10. Documentation, measured evidence, and status updates.

Out of scope (later phases): streaming, structured-output validation, terminal
approval, process/file effects, `koru shell` (2E), and Git (3).

---

## Specification

Normative; `MUST`/`MUST NOT` are hard, `SHOULD` allows a recorded justification.

### S1. Bounded JSON codec

- `parse(bytes, limits) -> Result<JsonValue>` MUST accept RFC 8259 JSON: objects,
  arrays, strings with `\" \\ \/ \b \f \n \r \t` and `\uXXXX` including surrogate
  pairs, numbers, booleans, and null. Input MUST be valid UTF-8.
- It MUST reject trailing data, unescaped control characters, invalid escapes,
  lone surrogates, `NaN`/`Infinity`, duplicate object keys, and any value beyond
  the JSON depth, element, or byte limits.
- Numbers without a fraction or exponent that fit `MAX_SAFE_INTEGER` parse as
  `Integer`; every other finite number parses as `Number`. Values outside the safe
  range that cannot be represented exactly MAY be `Number` only if finite.
- `emit(&JsonValue) -> String` MUST produce compact canonical JSON with sorted
  object keys, MUST escape controls, and MUST round-trip through `parse`.
- The codec MUST be used for protocol payloads and cache files; no provider or
  cache type leaks into `src/lua`.

### S2. Transport

- `Transport` is blocking and object-safe: request is method, absolute URL,
  headers, and bounded body bytes; response is status, bounded headers, and
  bounded body bytes. Adapters MUST NOT construct sockets directly.
- The production transport MUST apply connect and read timeouts, a maximum
  response size, a bounded redirect count that never forwards credentials to a
  different host, and a `koru/<version>` user agent. TLS roots MUST come from a
  pinned webpki root set, not the ambient environment.
- Transport failures MUST be classified (timeout, connect, TLS, status, body) and
  normalized into `ServiceError` without leaking headers, URLs with credentials,
  or bodies.
- The fixture transport MUST be deterministic and replayable for tests.

### S3. Protocol layer

- The first protocol is Chat Completions. The request builder MUST send `model`,
  ordered `messages`, `tools` in the function-calling shape, and bounded optional
  `max_tokens` and effort fields. Tool JSON schemas MUST come from the compiled
  `JsonSchema`, not raw Lua values.
- The response parser MUST extract text, tool calls (name, arguments, call id),
  normalized finish reason, usage, and request id, and MUST reject malformed or
  oversized responses with `ProviderFailure`.
- `ToolCall.id` becomes a bounded opaque string: providers assign string ids and
  Koru MUST NOT reinterpret them. Retry and replay tracking key on that string.
- `FinishReason` maps provider values to `stop`, `length`, `tool_calls`,
  `content_filter`, or `other`. Unknown usage stays `nil`.

### S4. Adapters

- `DeepSeekAdapter` implements `AiService` and `CatalogSource` over the transport
  with its own identity and credential variable.
- `OpenCodeAdapter` is constructed per identity (`opencode`, `opencode-go`) with
  separate endpoints, the documented `/zen/` paths, and its own catalog. Both use
  `OPENCODE_API_KEY`; Go additionally requires an active subscription, and a key
  alone does not prove entitlement.
- Go requests, including tool-loop requests and retries, MUST carry a stable
  `x-opencode-session` for the conversation.
- Each adapter reports `ServiceCapabilities`; a declared variant absent from the
  adapter mapping MUST fail preflight rather than being dropped.
- Adapter construction receives the immutable selection (service, model, variant,
  credential). Lua MUST NOT override provider, model, or variant in v1.

### S5. Catalog cache and selection

- `koru model update` fetches through the adapter and writes a per-service cache
  under `$XDG_CACHE_HOME/koru/catalog/<service>.json` containing a schema version
  and fetch time. The cache is disposable: corrupt or unreadable cache data is
  reported and refetched, never fatal.
- Cache writes MUST use a same-directory temporary file and atomic replacement
  under the shared lock. Refreshing one service MUST replace only that service and
  MUST retain its previous entries when the fetch or parse fails.
- Selection MUST be checked against the best available catalog; a selected model
  that is removed or unavailable MUST be reported, never silently substituted. A
  model change MUST clear a variant that is invalid for the new selection with a
  notice.
- Live metadata is advisory: a runtime requirement absent from cache MUST NOT be
  treated as supported.

### S6. Retries and budget

- Only explicitly classified transient failures (connect, timeout, 5xx, 429 with a
  bounded `Retry-After`) MAY be retried, and only within the original per-request
  attempt budget and the whole-command deadline.
- Every attempt charges `model_requests`; a retry storm MUST stop at the budget.
- A request MUST NOT be replayed after any tool call has executed. An ambiguous
  failure after effects MUST be reported as uncertain, never described as rolled
  back.

### S7. Hardening

- A shared `FileLock` (extracted from the config module) MUST provide bounded
  waiting and stale-lock detection; config and catalog cache both use it. A lock
  that cannot be proven stale MUST fail with actionable guidance.
- Provider error text MUST pass through the credential redactor before it reaches
  a user-visible error.
- No prompt, diff, or tool output is logged by default; diagnostics go to stderr.
- The release size MUST be re-measured and recorded; a size regression SHOULD be
  explained.

---

## Task 1: Dependency spike and transport trait

Files: `Cargo.toml`, `src/transport.rs`, `src/transport/ureq.rs` or equivalent,
`src/lib.rs`, `tests/transport.rs`.

1. Confirm the chosen blocking TLS client is available from the configured mirror
   or local cache; record the exact version. Fallbacks, in order: another blocking
   rustls client, then a blocking feature of a mainstream client, each still with
   no async runtime.
2. Define the `Transport` trait, request/response types, limits, and error classes.
3. Implement the fixture transport and the production transport.
4. Tests: fixture replay, size caps, redirect and credential-forwarding rules,
   timeout classification, and header redaction.

## Task 2: Bounded JSON codec

Files: `src/json.rs`, `src/json/parse.rs`, `src/json/emit.rs`, `tests/json_codec.rs`.

1. Implement `parse` and `emit` with the S1 rules and limits.
2. Reuse `JsonLimits`; add limits for nesting, elements, and total bytes if
   missing.
3. Tests: acceptance matrix, every rejection class, surrogate pairs, duplicate
   keys, and a deterministic round-trip property test.

## Task 3: Protocol layer

Files: `src/provider/protocol.rs`, `tests/protocol.rs`.

1. Build Chat Completions requests from `AiRequest` and `ToolSpec`.
2. Parse responses into `AiResult` and tool calls; normalize finish reasons,
   usage, and request ids.
3. Change `ToolCall.id` to a bounded string and update the fake, bridge, and tests.
4. Tests: golden request/response payloads, malformed/oversized rejection, tool
   call ordering, and finish-reason mapping.

## Task 4: DeepSeek adapter and live catalog

Files: `src/provider/deepseek.rs`, `src/provider/mod.rs`, `tests/deepseek.rs`.

1. Implement `AiService` and `CatalogSource` over the transport.
2. Map catalog documents to the bounded catalog model, including advertised
   effort levels when present.
3. Tests: request headers and auth, tool-loop requests, catalog parse, error
   normalization, and redaction.

## Task 5: OpenCode adapters

Files: `src/provider/opencode.rs`, `tests/opencode.rs`.

1. Implement Zen and Go identities with their endpoints and `/zen/` paths.
2. Add `x-opencode-session` continuity across tool-loop requests and retries.
3. Tests: separate identities and credentials, session header stability,
   entitlement failure normalization, and variant mapping.

## Task 6: Catalog cache and selection checks

Files: `src/provider/cache.rs`, `src/main.rs`, `tests/catalog_cache.rs`.

1. Add the versioned per-service cache under the XDG cache directory.
2. Wire `koru model update` to the live adapters and cache; keep the fixture
   source for tests.
3. Report a removed selected model; clear an invalid variant with a notice.
4. Tests: atomic replace, corrupt-cache recovery, per-service refresh, failure
   preservation, and selection-vs-catalog checks.

## Task 7: Retries

Files: `src/provider/retry.rs`, `src/lua/bridge/drive.rs`, `tests/retry.rs`.

1. Classify transient failures and implement bounded retry inside the attempt
   budget and deadline.
2. Refuse to replay after a tool call has executed; report uncertainty.
3. Tests: retry success, budget exhaustion, deadline stop, no replay after tools.

## Task 8: Hardening

Files: `src/lock.rs` (extract), `src/config.rs`, `src/provider/cache.rs`,
`src/credentials.rs`, `src/error.rs`.

1. Extract the shared `FileLock` with stale detection; use it for config and
   cache.
2. Route provider error text through redaction; confirm no default logging of
   prompts or tool output.
3. Re-measure the release size and review the dependency weight.

## Task 9: Documentation and status

Files: `README.md`, `docs/DESIGN.md`, `docs/implementation.md`, `docs/providers.md`.

1. Document the transport, JSON codec, protocol mapping, retries, sessions, and
   cache lifecycle.
2. Update evidence, remaining decisions, and the next gate (2E terminal and
   `koru shell`).

---

## Gate evidence mapping

| DESIGN gate | This increment's evidence |
| --- | --- |
| Providers | Live catalog parsing, capability rejection, protocol payloads, tool/result schemas, variants, session continuity, normalized failures |
| Persistence (partial) | Versioned catalog cache with atomic replace and shared lock; stale-lock detection |
| Cancellation and budgets | Retry storms bounded by the request budget and deadline |
| Lua/AI bridge | String tool ids, unchanged preflight, no replay after effects |

Terminal, Git, and full persistence gates remain unaddressed here by design.

## Completion checks

- `just check` passes (fmt, Clippy `-D warnings`, tests, Rustdoc).
- `cargo build --locked --release` and record the stripped size.
- One adapter is exercised against fixtures end to end; a live smoke test is
  documented as opt-in and never part of `just check`.
- Conventional commits authored `wallyhao <wallyhao@qq.com>`, one logical change
  each, subject at most 72 characters.

## Risks and fallbacks

- The TLS client may be unavailable or too heavy. Mitigation: the Task 1 spike
  records the outcome and fallbacks before any adapter work; the `Transport` trait
  keeps the choice isolated.
- Provider payloads drift or differ from fixtures. Mitigation: golden fixtures
  plus an opt-in live smoke test; parsers reject rather than guess.
- String tool ids ripple through the bridge. Mitigation: one mechanical commit
  updates the contract, fake, bridge, and tests together.
- Retries could duplicate work after effects. Mitigation: the executor records
  completed tool ids and retries are forbidden once any tool has run.
- A live catalog can be stale or unavailable. Mitigation: cache is advisory,
  refresh preserves prior entries, and selection reports instead of substituting.

## Follow-on phases (sketch)

- **Phase 2E - Terminal approval and `koru shell`.** Terminal detection, approval
  adapter, prepared shell execution under context-bound limits, output truncation,
  and the reference command.
- **Phase 3 - Git.** Snapshot, plan, grouped publication, recovery, and
  concurrency behind their dedicated gates.
