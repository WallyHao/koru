# Implementation status

Updated 2026-09-25. Lua commands with model calls now run through the CLI; this is not a v1 release.

## Available now

- A single Rust crate with typed error categories and XDG config/cache/state paths resolved once at the CLI boundary.
- Filename-only command discovery. Reserved and malformed command names fail explicitly. Discovery never opens or evaluates command source.
- Bounded source capture with leading `-- koru-module: module.name` directives, transitive pure-Lua module closure, cycle/traversal/symlink/bytecode rejection, and framed SHA-256 identity. Capture stores the bytes it read; later changes to installed files do not alter an existing bundle. `koru --inspect NAME` displays this metadata, not a syntax verdict.
- A restricted Lua VM built from an explicit library allowlist (`table`, `string`, `utf8`, `math`); unsafe base globals and the standard loader are removed. A bundle-backed `require` reads only captured bytes and rejects undeclared or cyclic modules.
- Bounded declaration evaluation: instruction, memory, and wall-clock limits are charged through the shared `ExecutionContext`; the ledger is terminal even when a script catches the abort with `pcall`. `koru check [command]` uses the same bounded loader; without a command it validates every discovered command.
- A Rust-owned API version 1 declaration contract (`description`, `arguments`, `capabilities`, `tools`, `run`) with one validator shared by `check` and the runtime.
- Typed Lua/JSON conversion (`koru.json`): explicit null, tagged empty arrays/objects, safe integers, and rejection of cycles, mixed keys, non-finite numbers, invalid UTF-8, functions, and oversized values.
- A Koru-owned JSON Schema subset (`src/schema.rs`): `parameters` and `result` compile at declaration time, model-supplied tool arguments are validated before the callback runs, and declared results are validated before they are forwarded. Unsupported keywords and remote references are rejected, not ignored.
- A single-VM AI bridge at the library boundary: the workflow runs in a coroutine and suspends with `yield_with`; the VM owner drives the agent loop over bounded channels and dispatches declared tool callbacks serially, with nested AI calls rejected and model/tool budgets charged from the shared ledger. `koru.ai.ask_json` validates one bounded response with a compiled Koru JSON Schema before returning data to Lua. Structured requests use prompt-and-validate and do not claim provider-native JSON mode.
- A provider boundary (`docs/providers.md`): a versioned strict `config.toml` with atomic writes and a scoped writer lock, environment credentials with redaction, three service identities, a bounded catalog model with a live fetch and a disposable versioned cache, adapter capability preflight, a blocking `ureq`/rustls transport, DeepSeek and OpenCode Chat Completions adapters (OpenCode Go with session continuity), and bounded retries charged to the shared budget. `koru model`, `koru model <service>[/<model>]`, `koru variant`, and `koru model update <service>|--all` select, persist, and refresh.
- A bounded JSON codec (strict parse, canonical emit) over `JsonValue`, used for protocol payloads and cache files.
- A Rust-owned `ExecutionContext` with an immutable command/source identity, shared atomic resource reservations, hard ceilings, cancellation, and whole-command deadline state.
- Immutable prepared process and shell descriptions, escaped approval previews, host-owned in-memory exact direct-process grants, default denial, and one-use broker authorization. Shell scripts cannot use stored direct-process exemptions. Action inputs have preparation limits.
- A Linux-only approved process executor wired to `koru.shell.script`. The bridge prepares one immutable script and cwd, displays the escaped preview, requests a terminal decision, authorizes once through the broker, and runs only the approved action. The executor binds executable and cwd handles, strips inherited credentials, caps both output streams, and kills and reaps the process group on timeout or cancellation. File action descriptions exist, but file effects are not exposed to Lua.
- A Linux-only read-only Git snapshot and plan validator wired to `koru.git.snapshot` and `koru.git.validate`. The staged diff is read only after one broker approval, with Git configuration, external diff, textconv, and optional locking disabled. Bounded path-free change summaries and excerpts go to Lua; opaque change IDs are checked for exact coverage, commit messages are validated, and the resulting plan has a snapshot-bound ID. This is plan-only: no Git index, worktree, refs, or commit objects are written.
- `koru <command> [arguments]` validates positional arguments, fixes the configured provider/model/variant for the run, executes the loaded Lua command through its provider adapter, and renders the result. Text goes to standard output; the reference shell result streams captured stdout/stderr and reports the child status and exit code, and the reference commit plan prints its identifiers and groups. Unknown result shapes fall back to indented JSON. `examples/commands/ask.lua`, `examples/commands/shell.lua`, and `examples/commands/commit.lua` are installable examples. `koru shell TASK` accepts one bounded task; without it, the CLI prompts on an interactive terminal for one bounded line before provider setup. `koru commit` reads staged changes, requests a model grouping plan, and prints the validated plan.
- A terminal adapter owns prompts, approval, and output routing. It emits restrained ANSI color only when the target stream is an interactive terminal and `NO_COLOR` is unset and `TERM` is not `dumb`; redirected output stays clean. The task prompt enables the terminal's `IUTF8` flag so backspace removes whole multi-byte characters, and approval accepts `y`/`yes` (anything else denies).
- `just check` runs formatting, Clippy with warnings denied, tests, and Rustdoc.

## Deliberate limitations

- `koru shell` is Linux-only and requires approval for every action. Redirected sessions cannot provide approval. Detached processes that deliberately escape the child process group are outside the cleanup guarantee. `koru model update` fetches live metadata, but there is no opt-in live provider smoke test.
- `koru commit` only returns a plan. It does not create commits; guarded ref publication, preservation of private staged and unstaged tree state, signing/hooks policy, and durable recovery remain required before a publishing mode can be enabled. File effects are not implemented. The declared per-service variants are provisional until real catalogs provide them; streaming is not implemented.
- The bridge requires services to be cooperative: on a terminal state the owner detaches the worker instead of joining it, so an uncooperative in-process service can linger until it returns. Generated-token accounting is not implemented; turns are capped by the service and the request's `max_turns`.
- Source path symlinks are rejected during capture, but a concurrent filesystem writer can still race path checks and opens. Before source capture is used for authorization or execution, replace this with handle-relative traversal and prove the no-escape property.
- Process execution is supported only on Linux with `/proc/self/fd` and process-group signaling; other platforms fail explicitly. Stored direct-process exemptions exist only in memory. The shell API does not expose arbitrary environment additions.
- Resource ceilings are initial conservative values. Lua, model calls, shell effects, and captured process output use the shared ledger; generated-token accounting is not implemented.

## Measured evidence

- Release binary: 4,487,288 bytes (stripped, thin LTO, Rust 1.98.1; measured 2026-09-25 after the plan-only Git API safety checks). Measurement only, not a target.
- Bridge: nested async `yield_with` is driven by synchronous `Thread::resume`; no async runtime or tokio is added. `set_global_hook` is observed to interrupt an mlua-created thread.
- Transport: one pinned blocking HTTPS client (`ureq = 3.3.0`, rustls with webpki roots); redirects are disabled and requests/responses are bounded. Adapters are tested against recorded fixtures; no live call runs in `just check`.
- Retries: transient failures are attempted at most three times per turn, each retry reserving one shared model request; client errors and malformed bodies are not retried.
- Limits: an infinite Lua loop aborts within one 10,000-instruction hook quantum; the owner checks cancellation and the deadline every 5 ms while waiting; service and owner channels are bounded at capacity 8; a tool storm is bounded by the tool-call budget, not the queue.
- 221 tests cover suspend/resume, serial tool ordering, nested-call rejection, proposal validation before approval, approval timeout/EOF, budget exhaustion, bounded cancellation latency, child-process cleanup, two-stream output caps, uncooperative-service shutdown, JSON codec round trips and rejection, schema compile/validate matrices, capability preflight, config atomicity and concurrent writers, credential redaction, catalog parsing/cache/refresh, protocol payloads, adapter tool loops and sessions, retry classification, approved process execution, staged Git plan validation without mutation, and CLI workflow dispatch.

## Next implementation gates

1. Add private-tree preservation, signing/hooks policy, guarded ref publication, and durable recovery gates before enabling any commit publication workflow.
2. Add a durable, versioned permission store and journal storage with writer locking and crash recovery; extend the shared persistence layer accordingly.
3. Add revalidated file effects, an opt-in live provider smoke test, and streaming.

All user-visible text remains in English as specified by the design document.
