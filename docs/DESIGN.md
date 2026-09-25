# Koru Design

Status: revised implementation design, 2026-09-25. This document specifies intended v1 behavior; it does not claim that the runtime or its guarantees have been implemented. Prototype and release gates below must pass before the corresponding feature ships.

## Purpose and scope

Koru is a lightweight Rust CLI for repeatable AI-assisted workflows. One Lua file defines one user command. Rust owns command discovery, model access through Rig, terminal presentation, permissions, resource accounting, and operating-system effects. Lua owns workflow composition, conditions, and loops.

The reference commands are `koru shell` and `koru commit`. v1 supports DeepSeek and two OpenCode services: Zen and Go. Provider IDs are `deepseek`, `opencode`, and `opencode-go`; `opencode` means Zen, not every provider supported by the OpenCode application. Initial supported models are restricted to tested transports and capabilities.

The application uses colored, structured terminal output rather than a full-screen TUI. All user-facing prompts, help, errors, templates, and documentation are in English. Keep a single Rust crate initially; introduce additional crates only when an actual ownership, reuse, or build boundary requires them.

## Architecture and ownership

Dependencies point toward Koru-owned contracts. Rig, mlua, HTTP clients, Git command details, and terminal widgets remain behind adapters.

| Component | Owns | Must not own |
| --- | --- | --- |
| CLI and composition root | Argument parsing, typed configuration loading, component construction, exit codes | Workflow or provider internals |
| Workflow runtime | ExecutionContext, command lifecycle, scheduling, cancellation, shared budgets | Provider wire formats |
| Lua adapter | Restricted VM, module loader, coroutine scheduling, value conversion | Independent authorization or process creation |
| AI service | Capability checks, bounded agent execution, normalized results and errors | Terminal prompts or Lua-specific values |
| Provider adapters | Rig integration, protocol mapping, credentials, request/session headers | Workflow decisions |
| Capability and approval broker | Requested/granted permissions, prepared operations, authorization checks | Model-authored authorization |
| Process and file adapters | Validated execution, bounded I/O, filesystem operations, cleanup | Permission policy decisions |
| Git service | Repository snapshots, commit plans, private indexes, guarded publication, recovery | AI grouping decisions |
| Terminal adapter | Rendering, input, approval decisions, output routing | Executing the operation being approved |
| Configuration and state stores | Validated persistence, locking, atomic replacement, schema versions | Mutable global runtime configuration |

Lua requests an effect through Koru APIs; the Rust broker prepares and authorizes it; the responsible adapter executes the same prepared operation. AI tools use this same path. The Git service is a Rust domain service, while the Lua commit workflow requests analysis, proposes grouping, and presents results.

The composition root loads configuration once and passes typed values to services. Each command receives an immutable configuration and capability snapshot. Library components do not reread global configuration during execution.

## User files and persistence

Use `$XDG_CONFIG_HOME/koru`, falling back to `~/.config/koru`:

```text
koru/
  config.toml
  permissions.toml
  commands/
    shell.lua
    commit.lua
    lib/
      shared.lua
```

Top-level Lua filenames define command names. `commands/lib/` contains modules, not commands. Reserve built-in names such as `model`, `variant`, `check`, and `recover`; reject collisions with an actionable error.

Model catalogs belong under `$XDG_CACHE_HOME/koru` (fallback `~/.cache/koru`). Durable operation journals belong under `$XDG_STATE_HOME/koru` (fallback `~/.local/state/koru`); recovery data must not be treated as disposable cache.

Configuration, grants, catalogs, and journals carry schema versions. Persistent changes use a scoped writer lock, validation, a same-directory temporary file, and atomic replacement. Durable grants and journals additionally require flushing the file and containing directory where supported. Unsupported persistence semantics must produce an explicit error for operations requiring recovery guarantees.

Concurrent model/variant selection must not lose another writer's changes. A catalog update replaces only the requested service's validated cache; failure retains its previous cache. Catalog updates do not mutate the selected model. Report a removed or unavailable selected model without silently selecting a substitute.

## Command lifecycle and trust model

v1 runs locally installed scripts that the user has chosen to run. It does not promise hostile-code containment or provide automatic remote plugin installation. Restricted Lua APIs and authorization gates still apply to local scripts. Allowing a subprocess grants that process its ordinary operating-system access; the broker is not an OS sandbox.

A command definition contains `api_version = 1`, a description, argument declarations, capability requirements, requested permission exemptions, tool declarations, and `run(koru, args)`. Unsupported API versions fail before execution.

Lifecycle:

1. Discover command filenames without executing them.
2. Read the selected script and statically declared pure-Lua module dependencies into a bounded, immutable source bundle. Compute its digest.
3. Evaluate the declaration in a restricted VM with instruction, memory, and wall-clock limits. Model, process, data-file, and approval APIs are unavailable in this phase.
4. Validate the definition, arguments, JSON schemas, dependencies, tool names, and requested permissions using the same validator used by `koru check`.
5. Resolve user policy into granted permissions and check declared model capabilities. Copy immutable declarations and grants into Rust-owned values.
6. Create the execution context, identify the selected provider/model, and run the workflow.
7. On completion, failure, or cancellation, stop new effects and perform bounded cleanup and journal reconciliation.

The source bundle includes the transitive module closure declared before evaluation. Modules resolve only below the configured `commands/lib/` root; reject absolute paths, parent traversal, symlink escapes, native modules, and undeclared dependencies. Each source declares module dependencies in leading comment directives of the form `-- koru-module: module.name`; the loader parses these without evaluating Lua and captures the transitive closure before declaration evaluation. Reject unresolved dependencies and dependency cycles in v1. Runtime module loads read the captured bundle, not files that could change after authorization. Reject bytecode input in v1.

Construct the Lua environment from an explicit library allowlist. Do not expose unrestricted `os`, `io`, `package`, `debug`, native loading/FFI, `load`, `loadfile`, or `dofile`. Provide only the controlled module loader and Koru host APIs. If coroutines are exposed, every creation path must install the execution controls described below.

`koru check [command]` validates syntax and declaration behavior under the same bounded loader. It reads only command/module sources and required Koru metadata; it performs no workflow data-file, model, or process operations and grants no permissions. It cannot prove all runtime branches succeed. Detailed command help uses the validated declaration; top-level discovery does not evaluate every script.

## Permissions and prepared operations

A script's `RequestedCapabilities` and requested exemptions are requests, not grants. The Rust policy resolver produces `GrantedCapabilities` from user-owned policy. A command cannot authorize itself by returning a declaration table or modifying Lua globals.

By default:

- Each direct process invocation requires confirmation.
- Shell syntax always requires confirmation.
- Each workflow data-file read, write, or directory operation requires confirmation.
- AI tool exposure is an explicit, separate list for each agent run.
- Starting a workflow authorizes its declared AI disclosure to the selected provider; there is no prompt before every model request.

Users may persist exact direct-process exemptions in Koru's permission store after reviewing them. Grants bind the command identity, source-bundle digest, policy version, resolved executable, exact argument vector, normalized working directory, and applicable environment policy. A script/module change invalidates the matching stored grant. Requested permissions never widen an existing grant. File operations and shell syntax have no persistent exemption in v1.

`koru.shell.run({ "ls", "-la" }, { cwd = "." })` directly invokes a program with an argument vector. Resolve the executable and directory before authorization; execute that resolved path without a second PATH lookup. Match structured values exactly; aliases, shell expansion, and argument wildcards are not implicit grants. `ll` is an alias, so a portable script should request `ls -l`.

`koru.shell.script(text, options)` uses a known configured shell with fixed invocation semantics. The preview includes shell identity, exact script text, and directory. Model-generated shell text always uses this path and cannot use a direct-process exemption.

The broker owns immutable `PreparedAction` values. Terminal previews and execution consume the same value; authorization is scoped to its action ID, execution context, and preconditions. Lua cannot fabricate an approval token or modify an authorized action. A denial means no execution. A changed action or failed precondition requires a new preparation and approval.

For writes, include the intended bytes or their digest and a useful bounded diff. Resolve path identity before approval and revalidate at execution; use handle-relative operations and a defined symlink policy where supported. Creation checks the parent directory and expected destination state. A textually unchanged path does not establish unchanged file identity.

The broker controls subprocess environment inheritance. Strip provider credentials and internal authorization data from child environments by default; explicit environment additions are part of the prepared operation. Provider credentials are available only to the provider adapter. This limits accidental disclosure; an approved process still has the user's OS-level capabilities.

Built-in Git group execution is an explicit compound action: its preview enumerates private-index/object/journal writes and the exact guarded branch transition. One approval covers those fixed internal steps for one group. This is a deliberate specialization of the operation rule, not authorization to batch arbitrary Lua or shell operations. Reviewing the overall commit plan does not authorize every group; each group still requires its own broker approval.

## Lua API and data contracts

The stable boundary is Koru API version 1, not a Rig or mlua API. Minor changes may add optional fields; incompatible semantics require a new API version and migration guidance.

- `koru.ai.ask(prompt)` makes one logical tool-free model call and returns `AiResult`.
- `koru.ai.run({ prompt = ..., tools = { ... }, max_turns = ... })` runs a bounded agent loop and returns `AiResult`.
- `AiResult` includes `text`, normalized `finish_reason`, selected model identity, available token usage, and request identifiers. Missing provider usage is represented explicitly as unknown. Structured-result options attach a supported JSON schema and return validated `data`; plain text is not silently interpreted as trusted JSON.
- A turn is one model generation attempt that produces an answer or tool requests. Transport retries consume the request budget even when they do not complete a turn. Tool calls have a separate budget.
- `ProcessResult` includes exit code or terminating signal, bounded stdout/stderr, and explicit truncation indicators. A nonzero exit is a process result; spawn failure, timeout, and cancellation are typed failures.
- Fallible Lua-facing APIs return a value on success or `nil, error` on failure. Boundary validation converts malformed inputs into validation errors rather than panics. Unhandled Lua exceptions become contextual runtime failures.
- Errors contain a stable category/code, an English message, command/tool/action context, and a retryability hint. Rust retains original causes while redacting secrets. Distinguish validation, permission denial, unsupported capability, provider failure, tool failure, timeout, budget exhaustion, state conflict, recovery required, and cancellation.
- Cancellation and exhausted execution budgets are terminal context states. Catching or ignoring their Lua error does not re-enable effects.

Tool definitions include a unique name, description, parameter schema, optional result schema, and callback. Registration occurs in the declaration phase. Each agent receives an immutable subset by name; later registration or schema mutation is rejected.

Rust validates arguments before invoking Lua and validates any declared result schema before forwarding output. v1 supports a documented JSON Schema subset: objects, properties, required fields, arrays/items, strings, booleans, finite numbers, safe integers, null, enums, and bounded lengths/ranges. Unknown keywords and remote references fail validation. Provider adapters must report when a schema exceeds their supported subset.

Conversions are explicit: `koru.json.null` represents JSON null; tagged constructors distinguish empty arrays from empty objects. Objects have string keys, arrays are contiguous, and portable integer values stay within the exact JSON/number range of +/- (2^53 - 1); larger identifiers use strings. Reject mixed-key tables, cycles, nonfinite numbers, functions, and unsupported userdata. Enforce depth, element-count, and byte limits before allocating or forwarding large values.

## Scheduling, cancellation, and resource budgets

Use one Lua VM owner per command with a cooperative coroutine scheduler. Workflow execution and tool callbacks execute serially within that VM. Async Rust services communicate through bounded request/reply channels carrying owned Koru values and tool IDs.

An AI call suspends the calling Lua coroutine. The VM owner continues servicing tool invocations from that suspended agent run. It must not block the executor thread or retain a VM access guard while awaiting a provider response. Rig callbacks enqueue invocation requests rather than accessing arbitrary Lua references across threads.

v1 rejects AI calls from inside a model-invoked Lua tool callback; nested agent/model execution can be introduced only with explicit reentrancy and budget semantics. A model response containing several tool calls is handled serially in stable response order. Queue capacity and total tool calls are bounded.

Every phase shares an `ExecutionContext` containing run ID, immutable configuration/model/source snapshots, grants, cancellation state, deadline, and resource ledger. All nested API calls debit that ledger. Limits include:

- Whole-command wall time, including approval waits, plus shorter per-call deadlines.
- Total model requests, retries, generated-token allowance, turns, and tool calls.
- Total effect invocations, pending requests, and accumulated input/output bytes.
- Lua memory, instruction budget, module/source bytes, JSON depth, and collection sizes.
- Per-response, per-stream, and per-tool limits enforced before unbounded buffering.

Rust owns defaults and hard ceilings. Lua may reduce limits, never reset or increase the context budget. Reserve capacity before dispatch. Use provider token caps and conservative local accounting when usage is unavailable; byte/request bounds remain mandatory and Koru does not claim exact billing enforcement.

Install VM execution hooks for the main coroutine and every permitted coroutine, plus a VM memory limit. Declaration evaluation is subject to the same controls. Restrict exposed host functions to bounded or cancellable operations. An async timeout alone cannot preempt a synchronous Lua loop; the prototype must verify hook coverage, error-catching behavior, coroutine creation, and cancellation latency. If the chosen VM cannot satisfy these requirements, use an isolated worker process before claiming the bound.

Ctrl-C marks the context cancelled, stops new effects, closes pending work, and triggers bounded cleanup. Terminate and reap subprocesses using supported process-group/job mechanisms; define supported-platform behavior for descendants. Dropping a future or child handle alone is insufficient. No automatic replay of a side-effecting tool after an ambiguous failure.

Only retry explicitly classified transient provider failures within the original budget and deadline. Never silently replay an entire agent run after tools may have executed. Record completed tool/action IDs for the current run; unresolved external side effects must be reported as uncertain, not described as rolled back.

## Model selection, capabilities, and providers

No profile abstraction is used. `config.toml` stores the selected provider/model and optional variant, without credentials. Read `DEEPSEEK_API_KEY` and `OPENCODE_API_KEY` from the environment; these names are Koru conventions.

```text
koru model
koru model deepseek/<model>
koru model opencode-go/<model>
koru variant
koru variant high
koru model update deepseek
koru model update opencode
koru model update opencode-go
koru model update --all
```

Selection applies to both AI APIs; Lua cannot override provider/model/variant in v1. A model change clears an unsupported variant and prints a notice. The chooser lists only models with an implemented transport. A run fixes its selection and capabilities for its lifetime.

Keep three concepts separate:

1. Catalog metadata: service/model identity, advertised limits and capabilities, source, and fetch time.
2. Adapter capability: tested protocol, tool/schema support, structured output, variant mapping, session headers, and normalized response behavior.
3. Workflow requirements: features required by the declaration and by each actual AI call.

Preflight checks the intersection. Transport availability alone does not establish tool or structured-output support. Unknown capability is not treated as supported. Repeat checks at each call for runtime-dependent requirements. An unsupported feature fails before dispatch; do not silently drop tools, schema requirements, or variants. Any prompt-and-validate fallback must be explicit and tested.

DeepSeek's `GET /models` supplies model metadata, including advertised effort levels when present. Rig provides DeepSeek integration; verify the chosen pinned version's model-listing and metadata coverage rather than assuming every advertised field is preserved.

OpenCode Zen publishes `GET https://opencode.ai/zen/v1/models`; Go publishes `GET https://opencode.ai/zen/go/v1/models`. Keep separate service identities and caches. OpenCode models may use Responses, Chat Completions, Messages, or other protocols; only explicitly implemented mappings are eligible. Catalog model IDs do not automatically determine protocol compatibility.

Both services use `OPENCODE_API_KEY`. Go additionally requires an active subscription, and key possession does not prove entitlement. Go requests use its documented `/zen/go/v1/` paths. Send Koru's own user agent and a stable `x-opencode-session` for each Go conversation, including its tool-loop requests and retries. Provider/service prefixes remain part of model identity.

Catalog parsing is bounded and validated. Refresh failures preserve existing entries and report the affected service. Stale metadata does not guarantee current availability or entitlement; normalize provider errors without changing the selected model automatically.

## Terminal interaction and disclosure

Interactive mode requires stdin and stderr attached to a terminal. Selection prompts show the current choice, navigation, cancellation, and a final confirmation before persisting configuration. `koru model` selects service and then model; `koru variant` shows only valid variants.

Use restrained color, aligned labels, terminal-width wrapping, and short progress/tool status lines. Respect non-color terminals and emit no ANSI sequences to redirected output. Escape untrusted control characters in model text, paths, command previews, and subprocess diagnostics so they cannot alter an approval display.

The terminal adapter is the only component that asks for operation approval. Workflow explanations may precede its preview but must not introduce a second authorization prompt for the same action. Diagnostics and progress go to stderr; intended command output goes to stdout.

Noninteractive mode never waits for input. Already granted exact operations may run; an operation requiring confirmation fails with a stable permission error and actionable guidance. Required missing workflow arguments fail before a model request. There is no blanket `--yes` bypass in v1.

At workflow start, identify provider/model and the declared categories of data to be sent. Scripts must not include secrets in requests or logs by default. Keep credentials inaccessible to the Lua environment; do not expose unrestricted environment reads. Redact known credentials at logging/error boundaries, while recognizing that redaction cannot reliably detect every secret in arbitrary user data. Full prompts, diffs, and tool outputs are not logged by default.

## Reference command: koru shell

1. Obtain the task from declared command arguments or interactive input.
2. Request a structured proposal containing script text, working directory, and explanation through a compatible AI capability.
3. Validate proposal shape, field sizes, and directory before preparing an effect.
4. Submit it through `koru.shell.script`. The Rust broker shows the exact prepared shell action and asks once for approval.
5. On approval, execute with context-bound timeout/output limits and report exit status, bounded output, and truncation. On denial, perform no execution.

The model has no executable shell tool in this workflow. Its proposal is data until the broker authorizes the prepared operation.

## Reference command: koru commit

The Lua command obtains a typed staged-change snapshot from the Rust Git service, requests logical groups and Conventional Commit messages, and returns the proposal to that service for validation. AI never supplies executable Git commands. Repository inspection is itself a broker-prepared read action whose preview names the repository and staged-data scope; its approval covers the fixed internal status/index/diff reads, not arbitrary filesystem access.

Each staged change has an opaque ID backed by exact repository paths, object IDs, and modes. The model groups IDs; raw model-produced paths are not trusted operation targets. File-level grouping means complete staged changes for a path, not current working-tree file contents. Couple rename endpoints and related file/directory transitions into indivisible change units where necessary. Deletion is a valid change and does not require a file to exist in the working tree.

Every staged change belongs to exactly one group. Validate complete coverage, uniqueness, valid intermediate trees, message rules, and bounded plan size. Show the full plan before any publication. A project-specific commit rule must come from explicit validated configuration or a supported rule source, not a model guess.

### Supported repository scope

Initial automatic execution supports a normal non-bare repository with an existing symbolic HEAD on a local branch and a full ordinary index. Reject active merge, rebase, cherry-pick, revert, sequencer, unmerged entries, or conflicting Koru recovery state before mutation.

Detached/unborn HEAD, sparse/split indexes, linked worktrees, and submodule changes require dedicated fixtures before being enabled. An unsupported case receives an explanation and may use plan-only output if snapshot inspection is safe. Resolve actual Git paths through Git rather than assuming `.git` is a directory. Use NUL-safe machine output and preserve path bytes in Rust; unsupported path encoding at a Lua boundary fails explicitly.

### Snapshot and candidate construction

Capture repository identity, symbolic HEAD target, original commit H0, original index bytes/fingerprint, staged tree I, relevant index flags, and effective commit/signing policy. Disable optional index refreshes for inspection. Read a consistent snapshot under the appropriate short-lived index lock; never hold that lock while waiting for AI or user input.

For groups G1..Gn, construct trees T1..Tn from H0 by applying the approved groups' staged object IDs and modes. Tn must equal I. Use private temporary indexes and Git object operations; never populate a group by rereading working-tree files. Do not use path-based `git commit --only`, `git add`, stash, or reset as the preservation algorithm.

Keep the user's original index unchanged throughout Koru's execution. After publishing a prefix of the groups, the difference between the new HEAD and I represents the remaining staged changes; the difference between I and the working tree continues to represent unstaged changes. At full completion, HEAD's tree equals I, without requiring an index rewrite. This invariant depends on the validated change partition and unchanged user index, and is a mandatory test property.

Use commit-object creation with explicit parent, message, author/committer policy, and configured signing before publishing each group. Signing failure leaves the target branch unchanged for that group. Preserve required signing; never downgrade silently.

### Hooks and external behavior

Commit-object creation does not reproduce ordinary commit hooks automatically. v1 automatic execution rejects repositories with enabled applicable commit or reference-transaction hooks until a tested integration exists; offer plan-only behavior. Resolve effective hook paths, including configured hook directories. Never silently bypass expected hooks.

Freeze and recheck relevant Git configuration and resolved executable identity. Explicitly control Git environment, external helpers, diff/text conversion, and optional maintenance behavior. Any external signer or other supported helper belongs in the prepared operation and shares its budget. Compatibility with these behaviors is a release gate, not an assumption about arbitrary Git installations.

### Authorization, publication, and recovery

One Git execution plan has a stable ID and digest. It binds repository/branch identity, H0, I, index fingerprint, ordered groups, messages, and execution policy. Approve each prepared group through the broker; bind its approval to the expected parent and exact proposed result.

Before branch publication:

1. Persist a restricted-access journal containing plan identity, expected parent, candidate commit/tree, current phase, and completed groups. Flush it before any branch change.
2. Acquire the repository-specific Koru execution lock and Git's index lock for the short publication window. Revalidate the index snapshot and relevant state. Existing or changed state causes a conflict; never overwrite it.
3. Use a Git reference transaction that verifies symbolic HEAD still names the planned branch and updates that branch from the expected parent to the candidate. Require tested support for symbolic-reference verification in no-dereference mode and expected-old-value updates; do not substitute a check-then-unconditional-write sequence.
4. Release the index lock without replacing the original index. Record publication durably and report the resulting commit.
5. Prepare the next group against the published parent. Any new approval occurs outside Git locks.

All Git helper processes invoked while the index is locked must use private indexes and must not attempt to acquire or rewrite the real index. Lock ownership and cleanup must be explicit; never remove a lock merely because its filename exists.

The journal and Git ref update are not one atomic transaction. Recovery reconciles them: if the branch equals the candidate whose publication was pending, that group completed; if it equals the expected parent, publication did not occur; another value is a conflict requiring inspection. An ambiguous I/O failure must trigger reconciliation, not an unguarded retry.

`koru recover <operation-id>` first inspects and reports state. Resumption requires a matching repository, index, expected branch state, valid candidate objects, and renewed approvals for remaining groups. Verify each candidate's parent/tree/message against the plan. Missing candidates may be rebuilt from the snapshot if safe; never guess after concurrent changes. State journals use restricted permissions and contain no credentials.

On cancellation or failure, retain already published commits, leave remaining changes staged, stop further publication, and report completed/remaining groups plus the recovery ID. Do not rewrite completed history or reset user files automatically. Clean only Koru-owned temporary state after its status is known. Detached helper processes, hooks, and non-cooperating writers are outside a promise of global filesystem atomicity; supported concurrency behavior must be demonstrated with ordinary Git writers.

Automatic grouped commits ship only after the snapshot, publication, and crash-recovery gates pass. Before then, expose planning without branch mutation.

## Verification and implementation sequence

Tests assert boundary behavior and preservation properties, not internal mock call sequences. Use disposable repositories and fake provider transports for deterministic faults; explicitly authorized live-provider smoke tests supplement adapter fixtures.

| Gate | Required evidence |
| --- | --- |
| Declaration and conversion | Bounded check/load, shared validation, API-version rejection, module traversal/change handling, JSON ambiguity/cycle/depth handling |
| Permissions | Self-declared exemptions do not grant access; changed source invalidates grants; action preview equals execution; denial performs no effect |
| Lua/AI bridge | Suspended workflow can receive serial tool calls; no VM lock across waits; nested model calls are rejected; queue bounds hold |
| Cancellation and budgets | Infinite loops, coroutine creation, caught errors, repeated ask calls, retry storms, oversized streams, child cleanup, and approval timeout all stop within documented bounds |
| Providers | Catalog parsing, capability rejection, protocol payloads, tool/result schemas, variants, header/session continuity, and normalized failures |
| Git content preservation | Partial staging, add/delete/rename/mode changes, unusual filenames, independent groups, and intermediate trees preserve index/worktree invariants |
| Git recovery | Inject failure before/after journal flush, candidate creation, reference transaction, and completion recording; reconcile without duplicate publication or lost work |
| Git concurrency | Concurrent staging, branch movement/switching, lock contention, and policy changes stop stale plans safely |
| Persistence and terminal | Interrupted writes preserve old data, concurrent writers do not lose updates, redirected output is clean, and noninteractive operations never prompt |

Implementation order:

1. Define Koru-owned contracts, API versioning, source-bundle loader, grants, prepared actions, and ExecutionContext.
2. Prove the single-VM async bridge and cancellation/resource behavior in a minimal prototype; select and pin compatible Rig/mlua features and versions.
3. Implement provider capability checks and the complete `koru shell` workflow.
4. Implement Git snapshot/planning, then guarded publication and recovery behind its release gate.
5. Expand tested service/model coverage and tune measured resource defaults and binary size.

## Project conventions

These conventions adapt architectural principles from `/home/wallyhao/Workspace/project-conventions/src/python-project-conventions.typ` to Rust. Python-specific tool and package rules do not apply.

- Keep Cargo configuration, task recipes, documentation, and project metadata at the root. Rust source belongs under `src/`, documentation under `docs/`, and integration tests under `tests/`.
- Split modules by the ownership boundaries above. Provider and terminal types must not leak into command scripts or Koru's core contracts.
- Use Rust naming conventions and concrete types at public boundaries. Public modules/items carry useful Rustdoc; module headers state responsibility and invariants.
- Treat roughly 120 nonblank, noncomment lines as a review signal, not a forced split rule. Keep cohesive ownership intact and record why a larger module remains cohesive.
- Load and validate configuration once; pass typed dependencies explicitly. Do not hardcode user paths or secrets. Redact credentials in errors and logs.
- Use typed Result errors with preserved causes and actionable messages. Avoid unchecked panics on user input and external responses.
- Run `cargo fmt`, focused `cargo clippy`, and tests required by the changed boundary. Do not copy the Python toolchain or impose an arbitrary coverage percentage.
- Update English documentation with behavior changes. Use Conventional Commits with one logical change per commit and a subject of at most 72 characters.
- Keep Rig/mlua features limited to measured needs. Measure release binary size before setting a numeric target; size targets must not obscure ownership.

## Remaining implementation decisions

These choices are resolved in the corresponding increment, or still require evidence before their affected gate can pass:

- Resolved: the API version 1 declaration field spelling and the argument/help schema (`docs/declaration.md`). Ergonomic constructors remain open.
- Resolved: the Lua binding and backend are `mlua = 0.12.1` with vendored Lua 5.5.1 and an explicit library allowlist. The executor/channels are std bounded channels driven by synchronous `yield_with`/`Thread::resume`; the global hook covers mlua-created coroutines, so no worker process is needed for the declaration bounds. The Rig version and provider capability mapping remain open.
- Resolved: typed Lua/JSON conversion and the documented JSON-schema subset (`docs/lua-api.md`). Tool arguments and declared results are validated against compiled schemas before dispatch and before forwarding; provider adapters must still reject schemas beyond their own subset once they exist.
- Resolved (initial values only): execution and Lua defaults/hard ceilings. A terminal bridge run returns without joining its service thread, a completed service is reaped within a 250 ms grace, and the cancellation-latency target is 50 ms. Supported-platform process cleanup remains open; the declaration hook observes cancellation within one instruction quantum and the bridge checks cancellation each wait tick.
- Open: initial tested Zen/Go models, per-protocol variant mapping, and fallback metadata sources when catalog capability fields are absent.
- Open: minimum tested Git version for reference transactions, signing-helper compatibility, and subsequent hooks/worktree/index-format support.
- Open: measured release binary size target; the current stripped measurement is 1,565,136 bytes.

Permission authority, shared budgets, preview/execution identity, staged-snapshot preservation, and recovery reconciliation are required architectural contracts rather than optional follow-up work.

## References

- DeepSeek model listing: https://api-docs.deepseek.com/api/list-models/
- OpenCode Zen API and catalog: https://opencode.ai/docs/zen/
- OpenCode Go API and session guidance: https://opencode.ai/docs/go/
- Rig DeepSeek integration: https://docs.rs/rig/latest/rig/providers/deepseek/
- Rig dynamic tools: https://docs.rs/rig/latest/rig/agent/tool/struct.DynamicTool.html
- Lua embedding, memory limits, and execution hooks: https://docs.rs/mlua/latest/mlua/struct.Lua.html
- Async timeout limitations: https://docs.rs/tokio/latest/tokio/time/fn.timeout.html
- Process lifetime and cleanup: https://docs.rs/tokio/latest/tokio/process/struct.Command.html
- Git commit and path semantics: https://git-scm.com/docs/git-commit
- Git commit-object creation and signing: https://git-scm.com/docs/git-commit-tree
- Git guarded reference transactions: https://git-scm.com/docs/git-update-ref
- Local conventions: `/home/wallyhao/Workspace/project-conventions/src/python-project-conventions.typ`
