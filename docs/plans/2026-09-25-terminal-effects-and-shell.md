# Terminal Effects and `koru shell` Implementation Plan

> **For Codex:** Use `${SUPERPOWERS_SKILLS_ROOT}/skills/collaboration/executing-plans/SKILL.md` to implement this plan task-by-task.

**Goal:** Make one installed Lua workflow executable through the CLI and ship `koru shell` with explicit approval, bounded execution, and no model-authored authorization.

**Architecture:** Keep the existing Koru-owned `ExecutionContext`, `PreparedAction`, broker, and provider boundary. Add a host-effects bridge to the single VM owner, a terminal adapter that returns a decision for the exact prepared action, and process/file executors that consume only one-use broker approvals. The reference Lua command requests a bounded shell proposal; Rust validates it before preparing an effect. No new crate or second permission path is needed.

**Tech Stack:** Rust 2024, existing `mlua = 0.12.1`, std bounded channels, existing blocking provider adapters, `clap`, disposable integration fixtures. Use a small platform-specific process-control dependency only if std cannot meet the descendant cleanup gate; record the measured reason first.

---

## Starting point and boundaries

Baseline: `d5b2f4f0c485f6cd67a424b8a382f6ca1afba11a` (`provider-2d-baseline-20260925`). `cargo test --locked --quiet` passes 179 tests. `src/main.rs` still returns `unsupported_capability` for workflows. `src/permissions/action.rs` prepares process and shell descriptions; `src/permissions/policy.rs` authorizes them in memory but has no executor. `src/lua/bridge.rs` exposes only `koru.json` and `koru.ai`.

Normative behavior comes from `docs/DESIGN.md`, especially “Permissions and prepared operations,” “Scheduling, cancellation, and resource budgets,” “Terminal interaction and disclosure,” and “Reference command: koru shell.” Keep the provider adapters behind `AiService`; do not add provider-specific types to Lua or the broker. Durable grants and Git journals belong to the following persistence/Git increments.

This increment includes direct process and shell actions, bounded data-file read/write/directory actions needed by the stable host API, terminal approval, noninteractive denial, CLI workflow dispatch, and `koru shell`. It does not include persistent exemptions, Git effects, streaming, or a broad `--yes` switch. Default confirmation for every process, shell, and data-file action remains in force.

## Contracts to settle before implementation

1. **Host API:** `koru.shell.run(argv, options)` and `koru.shell.script(text, options)` return `ProcessResult` or `nil, error`. Define the data-file API names and argument shapes in `docs/lua-api.md` before exposing them. Neither a Lua declaration nor model text grants an operation.
2. **Structured proposal:** The present `AiService` contract has no structured-output result. First add an explicit Koru-owned structured-result request and adapter capability check, or implement a documented prompt-and-validate fallback with bounded JSON parsing. Do not mark a model as structured-output capable merely because it supports Chat Completions. `koru shell` must reject malformed, oversized, or unsupported responses before preparing an action.
3. **Approval identity:** The terminal sees `PreparedAction::display_preview()` plus action ID, command identity, and any bounded diff. The executor consumes that same object through `ApprovedAction::begin`; no second path accepts raw Lua parameters.
4. **Filesystem identity:** Treat the current `canonicalize`-then-open behavior as insufficient. Record resolved executable/cwd identity at preparation, then use handle-relative access and recheck identity at dispatch. If the target platform cannot provide the required no-escape guarantee, fail explicitly rather than claiming it.
5. **Subprocess lifetime:** A timeout/cancel must terminate and reap the child and supported descendants, then return bounded diagnostics. Define the supported Linux behavior and fixture before implementation. Provider credentials and internal authorization values never enter the child environment by default.

## Task 1: Freeze observable CLI and Lua contracts

**Files:** Modify `docs/lua-api.md`, `docs/declaration.md`, `docs/implementation.md`; test `tests/cli.rs`, `tests/declaration.rs`.

1. Write fixture declarations for an argument-taking command and for the installed `shell.lua`; specify required argument behavior in interactive and redirected sessions.
2. Add failing CLI tests: a valid installed command reaches execution instead of `unsupported_capability`; a missing required task in redirected mode fails before provider construction; unknown command and invalid declaration keep stable typed errors.
3. Document exact `ProcessResult` fields (`exit_code` or `signal`, `stdout`, `stderr`, truncation flags), error categories, shell/file call shapes, and output routing. Keep all user-facing text in English.
4. Run `cargo test --locked --test cli --test declaration`; expect the new execution tests to fail for the current unsupported path. Commit the contract and tests (`test: define workflow execution contract`).

## Task 2: Extend prepared actions without duplicating policy

**Files:** Modify `src/permissions/action.rs`, `src/permissions/policy.rs`, `src/runtime/budget.rs`; create `src/permissions/path.rs` if cohesive path logic needs it; test `tests/permissions.rs`.

1. Add failing tests for prepared file read/write/directory operations, bounded write bytes and preview diff, source digest/context binding, one-use approval, denial with zero effects, and changed target identity requiring a new approval.
2. Add Rust-owned operation variants and preconditions. For writes, bind intended bytes or digest, expected destination state, parent identity, and symlink policy. For process/shell, bind executable and cwd identity plus exact argv/script and environment policy.
3. Change `ApprovedAction` so the executor obtains an owned approved operation exactly once; avoid a public API that returns a raw action usable after the one-use check. Reserve effect and byte budgets before dispatch.
4. Run `cargo test --locked --test permissions`; expect pass. Commit (`feat: bind prepared effects to checked identities`).

## Task 3: Implement the terminal decision adapter

**Files:** Create `src/terminal.rs` and `tests/terminal.rs`; modify `src/lib.rs` and `src/main.rs`.

1. Write tests using injected input/output handles for approve, deny, EOF, Ctrl-C, redirected stdin/stderr, escaped control characters, and width-limited preview rendering. Assert diagnostics go to stderr and redirected output has no ANSI escapes.
2. Implement one terminal-owned `decide(&PreparedAction) -> Decision` entry point. Interactive approval requires both stdin and stderr to be terminals; noninteractive mode immediately returns `Deny` with actionable guidance.
3. Make the preview render the exact prepared operation once. Never let Lua display substitute for the broker preview or ask a second confirmation for the same action.
4. Run `cargo test --locked --test terminal --test permissions`; expect pass. Commit (`feat: add terminal approval decisions`).

## Task 4: Execute direct and shell processes safely

**Files:** Create `src/effects/process.rs` and `src/effects/mod.rs`; modify `src/lib.rs`, `src/permissions/action.rs`, `src/runtime/budget.rs`; test `tests/process_effects.rs`.

1. Add failing disposable-process tests for exact argv (no shell expansion), fixed configured shell semantics, executable/cwd replacement after approval, clean child environment, reviewed explicit additions, nonzero exit as a result, spawn failure as typed error, stdout/stderr truncation, deadline, Ctrl-C, and child/descendant reaping.
2. Define a `ProcessExecutor` that accepts only an `ApprovedAction` plus its `ExecutionContext`. Revalidate identity from handles before spawn; never perform a fresh PATH lookup. Give the child a minimal environment and strip `DEEPSEEK_API_KEY`, `OPENCODE_API_KEY`, and internal authorization values.
3. Drain stdout and stderr concurrently into fixed caps; account bytes before retaining them. On cancel/timeout stop the process group, wait for reap within a bounded cleanup period, and return a typed terminal failure. Document unsupported platform behavior rather than silently weakening it.
4. Run `cargo test --locked --test process_effects --test permissions`; expect pass. Commit (`feat: execute approved bounded processes`).

## Task 5: Execute data-file actions through the same broker

**Files:** Create `src/effects/file.rs`; modify `src/permissions/action.rs`, `src/effects/mod.rs`; test `tests/file_effects.rs`.

1. Add failing tests for read/write/list approval, denial with no access, symlink and parent replacement between preview and execution, create-vs-existing state conflict, exact intended bytes, bounded diff, atomic write behavior, and output/byte budgets.
2. Implement handle-relative traversal and revalidation on supported platforms. Reads return bounded bytes. Writes use a same-directory temporary file and atomic replacement where that is the documented operation; do not overwrite a changed target. Directory operations remain individually approved.
3. Route all file effects through `PreparedAction`, `Broker`, and one-use `ApprovedAction`; do not expose arbitrary file handles to Lua.
4. Run `cargo test --locked --test file_effects --test permissions`; expect pass. Commit (`feat: execute approved data-file actions`).

## Task 6: Bridge host effects into the single Lua VM

**Files:** Modify `src/lua/bridge.rs`, `src/lua/bridge/drive.rs`, `src/lua/command.rs`; create `src/lua/bridge/effects.rs`; test `tests/bridge.rs`, `tests/lua_effects.rs`.

1. Add failing tests for direct and shell calls from a workflow, denied effects returning `nil, error`, malformed arguments becoming validation errors, multiple serial effects, catching an error without resetting a terminal budget, and a tool callback requesting an effect without bypassing the broker.
2. Extend the existing owner-driven `yield_with` protocol with an owned `EffectRequest`. While the coroutine is suspended, Rust prepares, obtains the terminal decision, authorizes, executes, and resumes with a bounded result. Preserve the rule that the VM owner never blocks while holding a VM access guard across provider work.
3. Convert `ProcessResult` and file results at one Koru-owned boundary. Reject Lua values that exceed JSON/action limits before they become Rust operations.
4. Run `cargo test --locked --test bridge --test lua_effects`; expect pass. Commit (`feat: bridge Lua effects through the broker`).

## Task 7: Wire real workflow CLI dispatch

**Files:** Modify `src/main.rs`, `src/lua/command.rs`, `src/provider/mod.rs`; test `tests/cli.rs`, `tests/workflow_cli.rs`.

1. Add failing integration fixtures for command discovery, declaration validation, argument parsing, immutable config/model/source snapshot, missing credential, capability preflight, successful workflow, noninteractive approval denial, and cancellation. Use a fixture `AiService`; do not call a live provider in ordinary tests.
2. Build one composition root path: capture source, validate declaration/arguments, resolve policy snapshot, create `ExecutionContext`, build selected adapter, and run the loaded command with the terminal/effect host. Load config once and keep provider credentials behind the adapter.
3. Ensure errors and progress go to stderr, intended workflow result to stdout, and exit status is nonzero for typed failures. Replace the generic `unsupported_capability` branch only after these gates pass.
4. Run `cargo test --locked --test cli --test workflow_cli`; expect pass. Commit (`feat: run installed Lua workflows from CLI`).

## Task 8: Ship the reference `koru shell` command

**Files:** Create `examples/commands/shell.lua` or a documented installable template under `assets/commands/shell.lua`; modify `src/ai/mod.rs`, `src/provider/protocol.rs`, `src/provider/deepseek.rs`, `src/provider/opencode.rs`, `src/lua/bridge/options.rs` as required by the chosen structured-result contract; test `tests/shell_workflow.rs`, provider fixture tests.

1. Write failing tests for a valid bounded proposal (`script`, `cwd`, `explanation`), malformed JSON/schema, oversized script, invalid cwd, provider lacking the needed result capability, refusal/empty answer, approval denial, successful execution, and nonzero exit reporting. Assert the model is offered no executable shell tool.
2. Implement the smallest explicit structured-result path. If using prompt-and-validate fallback, name it in API/docs, bound parse and field sizes, and test it against all three provider fixture adapters. Do not silently parse arbitrary `AiResult.text` as trusted data.
3. Have the Lua command obtain the task from a declared argument or interactive input, request the proposal, validate it, show the explanation, and call `koru.shell.script`. Rust remains the authority for cwd resolution, prepared preview, approval, and execution.
4. Run `cargo test --locked --test shell_workflow --test deepseek --test opencode`; expect pass. Commit (`feat: add approved shell workflow`).

## Task 9: Finish verification and documentation

**Files:** Modify `README.md`, `docs/DESIGN.md` only for resolved decisions, `docs/implementation.md`, `docs/lua-api.md`, and the reference command installation docs; update `justfile` only if a required gate changes.

1. Run `just check` and record actual test count and any platform-specific skips. Run focused fault fixtures for approval timeout, swapped paths, oversized streams, process descendants, repeated AI requests, cancellation, and noninteractive mode.
2. Build with `cargo build --locked --release`, measure stripped binary size, and record any dependency increase. An opt-in live provider smoke test may be run only with explicit credentials; fixture tests remain the release gate.
3. Update implementation status with what is actually supported, remaining persistence and Git gates, and exact `koru shell` installation/use examples. Correct stale status text in `README.md` and `docs/lua-api.md`.
4. Commit documentation separately (`docs: record shell execution gate evidence`). Leave the worktree clean and tag the completed increment only after the gate evidence is recorded.

## Release gate

The increment is complete only when: approval previews and execution consume the same immutable action; denial causes no effect; changed path identity conflicts; no child inherits provider credentials; bounded output and cleanup hold under cancellation; malformed model output cannot become a shell action; `koru shell` works through the normal CLI; and `just check` passes. Any unsupported platform/path semantics must fail explicitly and be listed in `docs/implementation.md`.
