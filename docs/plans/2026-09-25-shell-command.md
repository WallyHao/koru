# Koru Shell Command Implementation Plan

> **For Codex:** Use `${SUPERPOWERS_SKILLS_ROOT}/skills/collaboration/executing-plans/SKILL.md` to implement this plan task-by-task.

**Goal:** Ship `koru shell` as an installed Lua command that gets a bounded model proposal and executes its exact shell text only after one broker-owned terminal approval.

**Architecture:** Reuse the CLI workflow path, `ExecutionContext`, existing provider adapters, `PreparedAction`, broker, terminal adapter, and Linux process executor. Add a Koru-owned validated JSON proposal option and a host-effect request to the single-VM bridge; the model produces data and never receives an executable shell tool. The terminal previews the same immutable action that the executor consumes.

**Tech Stack:** Rust 2024, mlua 0.12.1, existing std channels, JSON codec/schema, DeepSeek/OpenCode fixture transports, disposable process fixtures. No new AI framework, TUI, credential store, or generic plugin executor.

---

## Baseline and delivery boundary

Base commit: `d2d6c33` (2026-09-25). `koru <command>` executes Lua/AI workflows; `src/terminal.rs` and `src/effects/process.rs` exist but are not wired to Lua. `src/lua/bridge.rs` currently suspends only for AI requests. `src/permissions/action.rs` and `policy.rs` own preparation and one-use authorization. `docs/plans/2026-09-25-terminal-effects-and-shell.md` records the broader effects plan; this plan narrows the next release to `koru shell`.

Keep `DEEPSEEK_API_KEY` and `OPENCODE_API_KEY` environment-only. `config.toml` continues to store provider/model/variant only. The shell workflow must not invent a second approval prompt or pass model text to a process before broker approval.

**Release boundary:** `koru shell` accepts a quoted task argument or reads one bounded task interactively. In noninteractive mode a missing task fails before any provider request. A valid model proposal contains only `script`, `cwd`, and `explanation`; every field has a finite byte limit. Rust resolves the shell and working directory, prepares the exact action, displays its escaped preview, obtains one decision, then runs or denies it. `ProcessResult` reports status, bounded stdout/stderr, and truncation flags. The first release supports Linux only; other platforms fail before asking approval.

## Task 1: Freeze the proposal and host API contract

**Files:** Modify `docs/lua-api.md`, `docs/declaration.md`, `src/ai/mod.rs`, `src/lua/bridge/options.rs`; test `tests/bridge.rs`, `tests/shell_workflow.rs`.

1. Write failing fixtures for an explicit `koru.ai.ask_json({ prompt, schema, mode = "prompt_validate" })` call. Its return is validated JSON data, never unvalidated `AiResult.text`. Define error codes for malformed JSON, schema mismatch, refusal/empty reply, oversize response, and unsupported schema.
2. Run `cargo test --locked --test bridge --test shell_workflow`; expect the new API fixtures to fail before a provider request is changed.
3. Specify the Koru-owned fallback: ask the selected adapter for plain text with a strict JSON instruction, parse with `src/json/parse.rs`, validate with the compiled `JsonSchema`, and return only validated data. This is explicitly `prompt_validate`, not a claim of provider-native structured output. Bound prompt and response bytes in the shared ledger.
4. Document `koru.shell.script(text, { cwd = ... }) -> ProcessResult | nil, error` and the exact `ProcessResult` fields. No arbitrary environment additions for model-proposed scripts. Run the focused tests and commit `docs: define shell proposal and effect contract`.

## Task 2: Make approval and process execution meet the release gate

**Files:** Modify `src/terminal.rs`, `src/effects/process.rs`, `src/permissions/action.rs`, `src/permissions/policy.rs`, `src/runtime/budget.rs`; test `tests/terminal.rs`, `tests/process_effects.rs`, `tests/permissions.rs`.

1. Add failing fixtures for approval timeout while input blocks, EOF, denied approval with zero effects, changed executable/cwd after preview, redirected stdin/stderr, escaped control text, child and descendant cleanup, large simultaneous stdout/stderr, and budget exhaustion.
2. Run the three focused test targets and record the failures. Strengthen the existing executor only where the fixtures show a gap: Linux handle-bound executable/cwd, fixed shell invocation, clean child environment with reviewed PATH, process-group kill and reap, and bounded pipe draining.
3. Ensure `ApprovedAction::begin` is one-use and occurs immediately before dispatch. The approved action, not fresh Lua values, is the sole executor input. Keep approval waits inside the whole-command deadline.
4. Run `cargo test --locked --test terminal --test process_effects --test permissions`; expect pass. Commit `fix: close shell approval and process cleanup gaps`.

## Task 3: Add an effect request to the single-VM owner

**Files:** Modify `src/lua/bridge.rs`, `src/lua/bridge/drive.rs`, `src/lua/command.rs`; create `src/lua/bridge/effects.rs`; test `tests/bridge.rs`, `tests/lua_effects.rs`.

1. Write failing tests where a workflow calls `koru.shell.script`, is suspended, receives a denied or approved result, and then continues. Test malformed options, a second call, cancelled context, and a tool callback attempting the same effect. No callback may bypass the broker.
2. Define an owned `PendingRequest` enum for AI and effect requests; keep the VM owner serial. The owner prepares an action, invokes the terminal adapter, calls `Broker::authorize`, runs the process executor, and resumes Lua with bounded `ProcessResult` or `nil, error`.
3. Keep authorization tokens and process handles in Rust. Reject nested AI execution as before. Terminal context states remain terminal even if Lua catches an error with `pcall`.
4. Run `cargo test --locked --test bridge --test lua_effects`; expect pass. Commit `feat: route Lua shell effects through broker`.

## Task 4: Implement validated proposal generation

**Files:** Modify `src/lua/bridge.rs`, `src/lua/bridge/options.rs`, `src/schema.rs`, `src/provider/protocol.rs` only if required; test `tests/shell_workflow.rs`, `tests/deepseek.rs`, `tests/opencode.rs`.

1. Add red tests for valid proposal, extra/unknown fields, invalid `cwd`, script over the limit, missing explanation, JSON code fences, trailing prose, invalid UTF-8, and provider errors. Assert no effect request is prepared for any invalid answer.
2. Compile the fixed proposal schema once in Rust. Validate before converting data back to Lua. Escape any explanation before terminal rendering. Keep tool exposure empty for the proposal call.
3. Confirm all three provider fixture adapters can carry the prompt and return text through the fallback. Do not change adapter capability flags to `structured_output = true`.
4. Run the focused tests; expect pass. Commit `feat: validate shell proposals before preparation`.

## Task 5: Add the reference command and CLI input

**Files:** Create `examples/commands/shell.lua`; modify `src/main.rs`, `src/terminal.rs`, `README.md`, `docs/lua-api.md`; test `tests/cli.rs`, `tests/shell_workflow.rs`.

1. Add failing CLI fixtures for `koru shell "task"`, missing task on redirected input, bounded interactive task input, denial, successful approved execution, nonzero exit, and selection/credential failures before proposal dispatch.
2. Make the command's optional `task` argument use the CLI value or one terminal-owned bounded input call. In noninteractive mode, fail with `validation` before contacting a provider. Do not add a blanket `--yes` bypass.
3. Install `shell.lua` as an example/template. It calls `ask_json`, checks the validated fields, shows a short explanation, and calls `koru.shell.script`; it never offers a shell tool to the model. Document installation next to `ask.lua`.
4. Run `cargo test --locked --test cli --test shell_workflow`; expect pass. Commit `feat: add approved koru shell command`.

## Task 6: Verify and record the release evidence

**Files:** Modify `docs/implementation.md`, `README.md`, `docs/DESIGN.md` only for resolved decisions; test fixtures from Tasks 1–5.

1. Run `just check`, count the actual tests, and verify the installed example with `koru check shell`. Run cancellation, approval timeout, changed-path, output-cap, and noninteractive fixtures separately; every denial must leave the fixture unchanged.
2. Run `cargo build --locked --release` and record the stripped binary size and any dependency change. Keep any real-provider smoke test opt-in and outside `just check`; credentials remain environment-only.
3. Record Linux-only support and known external-process limits. Ship only when preview/execution identity, no-effect denial, bounded cleanup, and malformed-proposal rejection all pass. Commit `docs: record shell release gate evidence`.

## Release gate

`koru shell` is complete only when a task reaches the selected model, its bounded proposal is schema-validated, the terminal approves the exact prepared script/cwd, and the broker-executed result is reported. A missing task, invalid proposal, denial, noninteractive session, timeout, cancellation, or changed target must execute no new shell action. `just check` and all focused fault fixtures must pass.
