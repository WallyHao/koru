# Implementation status

Updated 2026-09-25. The foundation in this repository is an implementation increment, not a v1 release.

## Available now

- A single Rust crate with typed error categories and XDG config/cache/state paths resolved once at the CLI boundary.
- Filename-only command discovery. Reserved and malformed command names fail explicitly. Discovery never opens or evaluates command source.
- Bounded source capture with leading `-- koru-module: module.name` directives, transitive pure-Lua module closure, cycle/traversal/symlink/bytecode rejection, and framed SHA-256 identity. Capture stores the bytes it read; later changes to installed files do not alter an existing bundle. `koru --inspect NAME` displays this metadata, not a syntax verdict.
- A Rust-owned `ExecutionContext` with an immutable command/source identity, shared atomic resource reservations, hard ceilings, cancellation, and whole-command deadline state.
- Immutable prepared process and shell descriptions, escaped approval previews, host-owned in-memory exact direct-process grants, default denial, and one-use broker authorization. Shell scripts cannot use stored direct-process exemptions. Action inputs have preparation limits.
- Portable integration fixtures cover the implemented boundaries. `just check` runs formatting, Clippy with warnings denied, tests, and Rustdoc.

## Deliberate limitations

- There is no Lua VM, declaration evaluation, `koru check`, model adapter, terminal approval prompt, OS effect adapter, persistence layer, or Git service yet. The CLI rejects workflow execution and `check` with a nonzero status.
- Source path symlinks are rejected during capture, but a concurrent filesystem writer can still race path checks and opens. Before source capture is used for authorization or execution, replace this with handle-relative traversal and prove the no-escape property.
- Prepared process paths are resolved at preparation time, but no executor revalidates file identity or operates on these actions. Stored exemptions exist only in memory and are not loaded or persisted. The process preview currently uses Rust debug rendering for environment additions; terminal presentation must be reviewed before the approval gate.
- Resource ceilings are initial conservative values. The Lua VM and provider adapters must reserve from this shared ledger and enforce their own per-call byte, stream, time, and cancellation bounds. No process or model request currently consumes it.
- The source loader's leading directive parser is intentionally lexical: it reads a comment header and ignores everything after the first code line. Runtime module loading must enforce that every requested module belongs to the captured closure.

## Next implementation gates

1. Pin and prototype mlua with explicit library allowlist, declaration memory/instruction/wall limits, and coroutine coverage. Implement `koru check` with shared command-schema validation.
2. Prove one-VM async agent/tool scheduling, cancellation, and typed JSON conversion under bounded channels. Then connect provider adapters and capability checks.
3. Add a terminal approval adapter and handle-relative, revalidated process/file execution with bounded output and cleanup. Add durable, versioned policy/config storage with writer locking and atomic replacement.
4. Ship `koru shell` only after its structured result, provider, permission, and cancellation gates pass.
5. Implement Git snapshot and plan-only `koru commit`, then guarded publication and recovery after the dedicated Git preservation and concurrency gates in DESIGN.md pass.

The first measured stripped release binary is 865,208 bytes on the current Linux host (Rust 1.98.1, thin LTO). This is a measurement, not a size target.

All user-visible text remains in English as specified by the design document.
