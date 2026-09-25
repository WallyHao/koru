# Implementation status

Updated 2026-09-25. The repository contains two foundation increments; it is not a v1 release.

## Available now

- A single Rust crate with typed error categories and XDG config/cache/state paths resolved once at the CLI boundary.
- Filename-only command discovery. Reserved and malformed command names fail explicitly. Discovery never opens or evaluates command source.
- Bounded source capture with leading `-- koru-module: module.name` directives, transitive pure-Lua module closure, cycle/traversal/symlink/bytecode rejection, and framed SHA-256 identity. Capture stores the bytes it read; later changes to installed files do not alter an existing bundle. `koru --inspect NAME` displays this metadata, not a syntax verdict.
- A restricted Lua VM built from an explicit library allowlist (`table`, `string`, `utf8`, `math`); unsafe base globals and the standard loader are removed. A bundle-backed `require` reads only captured bytes and rejects undeclared or cyclic modules.
- Bounded declaration evaluation: instruction, memory, and wall-clock limits are charged through the shared `ExecutionContext`; the ledger is terminal even when a script catches the abort with `pcall`. `koru check [command]` uses the same bounded loader; without a command it validates every discovered command.
- A Rust-owned API version 1 declaration contract (`description`, `arguments`, `capabilities`, `run`) with one validator shared by `check` and the future runtime. `tools` and `exemptions` are rejected until their runtimes exist.
- A Rust-owned `ExecutionContext` with an immutable command/source identity, shared atomic resource reservations, hard ceilings, cancellation, and whole-command deadline state.
- Immutable prepared process and shell descriptions, escaped approval previews, host-owned in-memory exact direct-process grants, default denial, and one-use broker authorization. Shell scripts cannot use stored direct-process exemptions. Action inputs have preparation limits.
- Portable integration fixtures cover the implemented boundaries. `just check` runs formatting, Clippy with warnings denied, tests, and Rustdoc.

## Deliberate limitations

- There is no model adapter, terminal approval prompt, OS effect adapter, persistence layer, or Git service yet. Workflow execution still returns a nonzero unsupported-capability error, and invocation arguments are not yet validated against declared schemas.
- Tool declarations and requested exemptions are rejected rather than accepted, because their runtimes do not exist.
- Source path symlinks are rejected during capture, but a concurrent filesystem writer can still race path checks and opens. Before source capture is used for authorization or execution, replace this with handle-relative traversal and prove the no-escape property.
- Prepared process paths are resolved at preparation time, but no executor revalidates file identity or operates on these actions. Stored exemptions exist only in memory and are not loaded or persisted. The process preview currently uses Rust debug rendering for environment additions; terminal presentation must be reviewed before the approval gate.
- The declaration VM retains a `run` function for the future runtime, but no execution path exists yet. Resource ceilings are initial conservative values and are consumed only by Lua evaluation so far; process and provider adapters must reserve from the same ledger.
- The source loader's leading directive parser is intentionally lexical: it reads a comment header and ignores everything after the first code line.

## Measured evidence

- Release binary: 1,436,224 bytes (stripped, thin LTO, Rust 1.98.1). Measurement only, not a target.
- Lua control: an instruction budget aborts an infinite loop within one hook quantum (10,000 instructions); a memory cap rejects large allocations as a bounded error; a 5 ms deadline aborts a running loop.
- Coroutine coverage: `set_global_hook` is observed to interrupt a thread created through `Lua::create_thread`. Lua-created coroutines are not exposed in the declaration sandbox.

## Next implementation gates

1. Prove one-VM async agent/tool scheduling, cancellation, and typed JSON conversion under bounded channels. Then connect provider adapters and capability checks.
2. Add a terminal approval adapter and handle-relative, revalidated process/file execution with bounded output and cleanup. Add durable, versioned policy/config storage with writer locking and atomic replacement.
3. Ship `koru shell` only after its structured result, provider, permission, and cancellation gates pass.
4. Implement Git snapshot and plan-only `koru commit`, then guarded publication and recovery after the dedicated Git preservation and concurrency gates in DESIGN.md pass.

All user-visible text remains in English as specified by the design document.
