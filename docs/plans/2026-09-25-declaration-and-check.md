# Koru Declaration Evaluation and `koru check` Implementation Plan

**Goal:** Deliver the second independently verifiable increment of DESIGN.md: bounded Lua declaration evaluation inside an explicit sandbox, a Rust-owned declaration contract shared by `koru check` and the future workflow runtime, and the `koru check [command]` CLI. No model, process, data-file, approval, persistence, or Git capability is enabled.

**Architecture:** A new Lua adapter (`src/lua/`) owns the restricted VM, the bundle-backed module loader, and value conversion. A new mlua-free contract module (`src/declaration.rs`) owns the validated declaration shape and is the single validator used by both `koru check` and the later runtime. The existing `ExecutionContext` supplies instruction, memory, deadline, cancellation, and terminal-state semantics. `main.rs` wires the `check` builtin; the composition root still constructs typed values once.

**Tech Stack:** Rust 2024, `mlua = "=0.12.1"` with features `["lua55", "vendored"]`, `clap` (existing), `sha2` (existing), `thiserror` (existing), `tempfile` (dev). `mlua-sys 0.12.0` pins the vendored source to `lua-src >= 551.0.0, < 551.1.0`, so Lua 5.5.1 is the only offline-buildable backend; both crates require Rust 1.88 and are already in the local cargo cache. The active registry mirror is `sparse+https://mirrors.bfsu.edu.cn/crates.io-index/`.

---

## Scope

In scope (DESIGN.md lifecycle steps 1-5 and the `Declaration and conversion` gate row):

1. Pin mlua and prove VM control coverage with executable evidence.
2. Construct a sandboxed VM from an explicit library allowlist; remove unsafe base globals; install only a controlled `require` backed by the captured `SourceBundle`.
3. Enforce instruction, memory, wall-clock, deadline, and cancellation bounds through the shared `ExecutionContext` ledger, with terminal states that Lua cannot reset.
4. Convert and validate a `return { ... }` declaration into Rust-owned types under strict field and size limits.
5. Implement `koru check [command]` using that same validator; it reads command/module sources and Koru metadata only, grants no permissions, and performs no model, process, or data-file operation.

Out of scope (deferred gates; do not claim in docs):

- Tool declaration execution/registration, request/reply channels, and the one-VM async bridge (gate 2).
- Lua-to-JSON conversion and JSON Schema subset validation (gate 2).
- Provider adapters, terminal approval, OS effect execution, durable grants/config, Git planning, recovery.
- Argument **value** validation (needs the runtime argument path); this increment validates argument **schemas** only.

---

## Frozen decisions for this increment

These resolve part of DESIGN.md's "Remaining implementation decisions" (exact declaration field spelling, argument/help schema, pinned mlua/backend). Record the outcome in `docs/declaration.md`, `docs/implementation.md`, and DESIGN.md's decision list when the increment lands.

Declaration (API version 1), all keys rejected if unknown:

```lua
return {
  api_version = 1,                     -- required integer; must equal 1
  description = "One sentence",        -- required string, 1..=1024 bytes, no control chars
  arguments = {                        -- optional ordered array, at most 32 entries
    { name = "task", type = "string", required = true, help = "..." },
    { name = "count", type = "integer", default = 1, min = 1, max = 10 },
  },
  capabilities = {                     -- optional table; only known keys
    direct_processes = false,          -- request only, never a grant
  },
  run = function(koru, args) end,      -- required function; not called by this increment
}
```

- Argument `type` is one of `string`, `integer`, `number`, `boolean`, `enum`. `enum` requires a non-empty `values` array of strings. `name` matches `[a-z][a-z0-9_]*`, at most 64 bytes, unique. `required` and `default` are mutually exclusive. Bounds: `min`/`max` for `integer`/`number`, `max_len` for `string`, `max_items` for future arrays.
- `tools` and requested `exemptions` are valid parts of the eventual v1 definition but are **rejected with `unsupported_capability`** until the AI bridge and durable exemption store exist. Rejecting them prevents a false "validated" verdict; document this explicitly.
- `koru` is undefined during declaration evaluation; the host API table is only passed when `run` is invoked later.
- `require` is a Koru-installed host function, not the standard loader. It resolves only names present in the captured `SourceBundle`, caches evaluated modules per run, and returns `true` when a module yields no value. It never touches the filesystem and never loads bytecode.
- Backend: vendored Lua 5.5.1 via `mlua = "=0.12.1"`, features `["lua55", "vendored"]`. Feature set stays minimal; do not enable `async`, `send`, `serde`, or `macros` until measured needs require them.
- `Cargo.toml` `rust-version` rises to `1.88` (mlua/mlua-sys minimum); adjust the README statement if needed.
- `koru check [command]`: with a name, validate that command; without a name, validate every discovered command in sorted order. Results on stdout, diagnostics on stderr, exit 0 only when all validations succeed, no ANSI when redirected, never interactive.

---

## Task 1: Pin mlua and prove VM controls

Files: `Cargo.toml`, `Cargo.lock`, `tests/lua_controls.rs`.

1. Add `mlua = { version = "=0.12.1", features = ["lua55", "vendored"] }`; run `cargo build --locked` and confirm the vendored Lua compiles with the available `cc` (no system Lua, no self-installed tool).
2. Add the Evidence tests below before building the production adapter; verify they fail to compile/behave until the adapter exists.
3. Prove and record:
   - `Lua::new_with(StdLib::TABLE | StdLib::STRING | StdLib::UTF8 | StdLib::MATH, ...)` yields a state where `os`, `io`, `package`, `debug`, `load`, `loadfile`, `dofile`, `print`, `collectgarbage`, `coroutine`, and `koru` are absent after the explicit strip step.
   - `set_memory_limit` turns an allocation bomb into `mlua::Error::MemoryError` without aborting the Rust process.
   - `set_global_hook(HookTriggers::new().every_nth_instruction(N), ..)` aborts `while true do end` within the expected instruction window; a `pcall` wrapper cannot re-enable execution once the ledger is terminal.
   - The wall-clock/deadline path aborts a slow chunk, and `context.cancel()` aborts a running chunk.
   - Whether the global hook is inherited by a coroutine created by Rust (`Lua::create_thread`) and by Lua's own `coroutine.create`. Record the measured result and cancellation latency.
4. Decision rule: if hooks do **not** cover Lua-created coroutines, keep `StdLib::COROUTINE` excluded from the declaration sandbox and record that gate 2 must either install `Thread::set_hook` on every thread or isolate execution in a worker process before claiming cancellation bounds.

Acceptance: the control suite passes, and the measured instruction abort point, memory ceiling, and cancellation latency are written into `docs/implementation.md`.

---

## Task 2: Sandboxed Lua adapter and bundle-backed module loader

Files: `src/runtime/budget.rs`, `src/lua/mod.rs`, `src/lua/sandbox.rs`, `src/lua/loader.rs`, `tests/lua_sandbox.rs`.

1. Extend `Limits` with `instructions: u64` and `lua_memory_bytes: u64`, and `Resources` with `instructions: u64`; extend the hard-ceiling validation and `Limits::default`. `lua_memory_bytes` is a VM cap, not a cumulative debit. Update the explicit `Limits` literals in `tests/runtime.rs`.
2. `Sandbox::new(context, limits)` constructs the VM with the explicit library allowlist, then removes every residual unsafe global. It stores an `Arc` control state with the `ExecutionContext` so the hook can debit the ledger and observe cancellation.
3. Hook policy: on each `every_nth_instruction(N)` tick, `context.reserve(Resources { instructions: N, .. }, now)` and `context.ensure_active(now)`. Failure returns `Err` from the hook, which aborts the chunk; the terminal ledger state is authoritative and cannot be reset by `pcall`. Set `set_memory_limit(limits.lua_memory_bytes)`.
4. `loader.rs` installs `require` as a Rust function over the `SourceBundle`; reject any name not in the captured closure with `ErrorCode::Validation`; cache per run; use `@{command}/lib/{name}.lua` chunk names for diagnostics. The loader must read only captured bytes, so a file changed after capture does not alter evaluation.
5. Map mlua errors to typed `KoruError`: terminal context state wins first (`BudgetExhausted`/`Timeout`/`Cancelled`); `MemoryError` maps to `BudgetExhausted`; syntax/declaration errors map to `Validation` with a bounded, escaped message. Discard tracebacks; never leak environment contents.

Acceptance: `tests/lua_sandbox.rs` passes, including the "changed file after capture does not affect evaluation" and "undeclared `require` rejected" cases.

---

## Task 3: Rust-owned declaration contract and shared validator

Files: `src/declaration.rs`, `src/lib.rs`, `src/lua/declaration.rs`, `tests/declaration.rs`.

1. `src/declaration.rs` (no mlua dependency): `CommandDeclaration`, `Argument`, `ArgumentType`, `ToolDeclaration` reserved stub, `Capabilities`, and `validate`-style constructors plus the numeric/string limits from "Frozen decisions". Keep public items documented; keep the module cohesive and record if it grows past the review signal.
2. `src/lua/declaration.rs` converts an evaluated `mlua::Value::Table` into `CommandDeclaration` with strict type, key, length, and depth checks; reject unknown keys and non-function `run` with `ErrorCode::Validation`; map `api_version != 1` to `ErrorCode::UnsupportedCapability`; map `tools`/`exemptions` to `ErrorCode::UnsupportedCapability` with actionable text.
3. `LoadedCommand` (in the adapter) owns the VM and the declaration so the future runtime can call `run`; `koru check` may drop it after validation. Provide one `LoadedCommand::load(bundle, context) -> Result<LoadedCommand>` entry point used by both `check` and the future runtime.
4. Tests: valid minimal and full declarations; missing/invalid `api_version` (including `2`); unknown keys at each level; wrong value types; duplicate or malformed argument/tool names; `required`+`default` conflict; bounds violations; missing/non-function `run`; `tools`/`exemptions` rejected as unsupported; oversized `description` and argument count.

Acceptance: `tests/declaration.rs` passes; the same validator is the only path used by the CLI.

---

## Task 4: `koru check` CLI, docs, and status

Files: `src/main.rs`, `tests/cli.rs`, `README.md`, `docs/declaration.md`, `docs/implementation.md`, `docs/DESIGN.md`.

1. In `run()`, treat a first positional `check` as the builtin: capture each selected command's bundle, create an `ExecutionContext` with `Limits::default()` and the bundle digest, load and validate the declaration, then print `"{name}: ok"` or `"{name}: {code}: {message}"`. At most one command name is allowed; more is a validation error. `model`, `variant`, `recover`, `check`, `help`, `list`, `inspect` remain reserved and return `unsupported_capability` when invoked before their runtimes exist. Keep `--inspect` unchanged in this increment; a reviewed clap subcommand structure is deferred to the `model`/`variant` increment.
2. `docs/declaration.md` becomes the authoritative English reference for API version 1: fields, types, limits, `require` semantics, and error codes, with the example above.
3. Update `README.md`: remove the "only captures bytes / does not validate declaration syntax" caveats, document `koru check`, and link this plan and `docs/declaration.md`.
4. Update `docs/implementation.md`: move the relevant lifecycle items into "Available now", record the measured release size and Task 1 measurements, and restate what remains deferred.
5. Update DESIGN.md's "Remaining implementation decisions" to mark the resolved items (field spelling, argument schema, pinned backend) and leave the still-open ones.

Acceptance: `tests/cli.rs` covers valid, invalid, unsupported-version, missing-command, and no-ANSI cases; `docs/declaration.md` matches the validator exactly.

---

## Completion checks

1. `just check` (format check, Clippy with warnings denied, all tests, Rustdoc with warnings denied).
2. `cargo build --locked --release` and record the stripped binary size in `docs/implementation.md` (measurement only, no target).
3. CLI smoke test against an isolated fixture directory: valid `check`, invalid declaration, `check` with no arguments over several commands, and `--inspect` unchanged.
4. Confirm `koru check` performs no filesystem write, model, or process operation and grants no permission; confirm redirected output has no ANSI sequences.
5. Keep conventional commits, one logical change each, authored `wallyhao <wallyhao@qq.com>`, subject at most 72 characters. Suggested sequence: `feat: sandbox and validate Lua command declarations`, then `docs: document declaration contract and check status`.

---

## Risks and fallbacks

- **Coroutine hook coverage** is the main empirical unknown; Task 1 decides it. Fallback: keep coroutine creation unavailable in the declaration sandbox and require per-thread hooks or a worker process in gate 2.
- **Vendored Lua build** needs a working C toolchain; `cc`/`gcc` are present on this host. Only Lua 5.5.1 sources are vendored by `mlua-sys 0.12.0`; do not request `lua54`.
- **Mirror-only network**: all needed crates are already cached; if a transitive build dependency is missing, stop and report rather than working around the environment.
- **Terminal-state integrity**: any hook or `pcall` path that could resume effects after exhaustion/cancellation is a release blocker; the ledger, not the Lua error, decides.
- **Rustdoc/line budgets**: every public item needs documentation and `unsafe_code` stays forbidden; split modules only at real ownership boundaries.

## Deferred release gates

Single-VM async bridge and typed JSON conversion; provider adapters and capability checks; terminal approval and revalidated process/file execution; durable versioned config/grants with locking; `koru shell`; Git snapshot, guarded publication, and recovery. None is claimed by this increment.
