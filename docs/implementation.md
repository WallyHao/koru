# Implementation status

Updated 2026-09-25. The repository contains three foundation increments; it is not a v1 release.

## Available now

- A single Rust crate with typed error categories and XDG config/cache/state paths resolved once at the CLI boundary.
- Filename-only command discovery. Reserved and malformed command names fail explicitly. Discovery never opens or evaluates command source.
- Bounded source capture with leading `-- koru-module: module.name` directives, transitive pure-Lua module closure, cycle/traversal/symlink/bytecode rejection, and framed SHA-256 identity. Capture stores the bytes it read; later changes to installed files do not alter an existing bundle. `koru --inspect NAME` displays this metadata, not a syntax verdict.
- A restricted Lua VM built from an explicit library allowlist (`table`, `string`, `utf8`, `math`); unsafe base globals and the standard loader are removed. A bundle-backed `require` reads only captured bytes and rejects undeclared or cyclic modules.
- Bounded declaration evaluation: instruction, memory, and wall-clock limits are charged through the shared `ExecutionContext`; the ledger is terminal even when a script catches the abort with `pcall`. `koru check [command]` uses the same bounded loader; without a command it validates every discovered command.
- A Rust-owned API version 1 declaration contract (`description`, `arguments`, `capabilities`, `tools`, `run`) with one validator shared by `check` and the runtime.
- Typed Lua/JSON conversion (`koru.json`): explicit null, tagged empty arrays/objects, safe integers, and rejection of cycles, mixed keys, non-finite numbers, invalid UTF-8, functions, and oversized values.
- A Koru-owned JSON Schema subset (`src/schema.rs`): `parameters` and `result` compile at declaration time, model-supplied tool arguments are validated before the callback runs, and declared results are validated before they are forwarded. Unsupported keywords and remote references are rejected, not ignored.
- A single-VM AI bridge at the library boundary: the workflow runs in a coroutine and suspends with `yield_with`; the VM owner drives the agent loop over bounded channels and dispatches declared tool callbacks serially, with nested AI calls rejected and model/tool budgets charged from the shared ledger. A deterministic fake service (`koru::ai::fake`) covers the gate. A terminal run returns without waiting for its service thread, and a completed service is reaped within a bounded grace.
- A provider boundary (`docs/providers.md`): a versioned strict `config.toml` with atomic writes and a scoped writer lock, environment credentials with redaction, three service identities, a bounded catalog model with a live fetch and a disposable versioned cache, adapter capability preflight, a blocking `ureq`/rustls transport, DeepSeek and OpenCode Chat Completions adapters (OpenCode Go with session continuity), and bounded retries charged to the shared budget. `koru model`, `koru model <service>[/<model>]`, `koru variant`, and `koru model update <service>|--all` select, persist, and refresh.
- A bounded JSON codec (strict parse, canonical emit) over `JsonValue`, used for protocol payloads and cache files.
- A Rust-owned `ExecutionContext` with an immutable command/source identity, shared atomic resource reservations, hard ceilings, cancellation, and whole-command deadline state.
- Immutable prepared process and shell descriptions, escaped approval previews, host-owned in-memory exact direct-process grants, default denial, and one-use broker authorization. Shell scripts cannot use stored direct-process exemptions. Action inputs have preparation limits.
- `just check` runs formatting, Clippy with warnings denied, tests, and Rustdoc.

## Deliberate limitations

- No terminal approval or effect executor exists, so `koru <command>` still returns a nonzero unsupported-capability error and the bridge is exercised through the library and the fake service. `koru model update` fetches live metadata, but there is no opt-in live smoke test yet.
- The JSON Schema subset is enforced and adapter capabilities are checked, but the declared per-service variants are provisional until real catalogs provide them, and streaming and structured-output validation are not implemented.
- The bridge requires services to be cooperative: on a terminal state the owner detaches the worker instead of joining it, so an uncooperative in-process service can linger until it returns. Generated-token accounting is not implemented; turns are capped by the service and the request's `max_turns`.
- Source path symlinks are rejected during capture, but a concurrent filesystem writer can still race path checks and opens. Before source capture is used for authorization or execution, replace this with handle-relative traversal and prove the no-escape property.
- Prepared process paths are resolved at preparation time, but no executor revalidates file identity. Stored exemptions exist only in memory. The process preview uses Rust debug rendering for environment additions.
- Resource ceilings are initial conservative values; only Lua evaluation and the bridge charge them so far.

## Measured evidence

- Release binary: 3,878,864 bytes (stripped, thin LTO, Rust 1.98.1). The 2C-to-2D growth (about 2.24 MB) is the rustls/ring/webpki-roots TLS stack needed for live HTTPS. Measurement only, not a target.
- Bridge: nested async `yield_with` is driven by synchronous `Thread::resume`; no async runtime or tokio is added. `set_global_hook` is observed to interrupt an mlua-created thread.
- Transport: one pinned blocking HTTPS client (`ureq = 3.3.0`, rustls with webpki roots); redirects are disabled and requests/responses are bounded. Adapters are tested against recorded fixtures; no live call runs in `just check`.
- Retries: transient failures are attempted at most three times per turn, each retry reserving one shared model request; client errors and malformed bodies are not retried.
- Limits: an infinite Lua loop aborts within one 10,000-instruction hook quantum; the owner checks cancellation and the deadline every 5 ms while waiting; service and owner channels are bounded at capacity 8; a tool storm is bounded by the tool-call budget, not the queue.
- 179 tests cover suspend/resume, serial tool ordering, nested-call rejection, budget exhaustion, bounded cancellation latency, uncooperative-service shutdown, JSON codec round trips and rejection, schema compile/validate matrices, capability preflight, config atomicity and concurrent writers, credential redaction, catalog parsing/cache/refresh, protocol payloads, adapter tool loops and sessions, and retry classification and budget integration.

## Next implementation gates

1. Add a terminal approval adapter and handle-relative, revalidated process/file execution with bounded output and cleanup. Ship `koru shell` only after its structured result, provider, permission, and cancellation gates pass.
2. Add a durable, versioned permission store and journal storage with writer locking and crash recovery; extend the shared persistence layer accordingly.
3. Add an opt-in live provider smoke test, streaming, and structured-output validation once the terminal/effect path is stable.
4. Implement Git snapshot and plan-only `koru commit`, then guarded publication and recovery after the dedicated Git preservation and concurrency gates in DESIGN.md pass.

All user-visible text remains in English as specified by the design document.
