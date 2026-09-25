# Koru Commit Command Implementation Plan

> **For Codex:** Use `${SUPERPOWERS_SKILLS_ROOT}/skills/collaboration/executing-plans/SKILL.md` to implement this plan task-by-task.

**Goal:** Ship `koru commit` first as a safe plan-only staged-change grouper, then enable per-group guarded commits with durable recovery after Git preservation and concurrency gates pass.

**Architecture:** Lua requests a typed staged snapshot and asks the selected model to group opaque change IDs and draft Conventional Commit messages. A Rust Git service owns repository inspection, validation, private-index tree construction, commit-object creation, journal reconciliation, and guarded reference publication. Repository reads and each group publication are broker-prepared actions; the model never supplies executable Git commands or trusted paths.

**Tech Stack:** Existing Rust 2024 crate, `std::process::Command` behind a Git adapter, bounded NUL-safe Git machine output, the existing Lua/AI bridge and broker, disposable Git repositories, versioned state under `$XDG_STATE_HOME/koru`. Keep credentials environment-only and do not add a generic Git framework or shell-based preservation algorithm.

---

## Baseline, sequence, and supported scope

Base commit: `d2d6c33` (2026-09-25). `koru <command>` can run Lua and model calls; no Git service, durable journal, or file effect executor exists. `docs/DESIGN.md` sections “Reference command: koru commit,” “Supported repository scope,” “Snapshot and candidate construction,” and “Authorization, publication, and recovery” are normative. Implement after the broker/effect bridge from the shell plan is available; reuse it rather than creating a Git-only approval UI.

Deliver in two explicit releases:

- **3A plan-only:** inspect a supported repository after approval, ask the model for grouping, validate coverage and messages, display the complete plan, and do not mutate the branch, index, or working tree.
- **3B execution/recovery:** only after the preservation, hooks/signing, reference-transaction, crash-recovery, and concurrency fixtures pass. Each group gets its own prepared broker approval; plan review alone never authorizes publication.

Initial automatic scope: ordinary non-bare repo, existing symbolic HEAD on a local branch, full ordinary index, no active merge/rebase/cherry-pick/revert/sequencer/unmerged entries or conflicting Koru journal. Detached/unborn HEAD, sparse/split index, linked worktree, submodule changes, and applicable hooks are plan-only or explicit unsupported cases until dedicated fixtures exist. Resolve Git paths with Git; do not assume `.git` is a directory. Minimum Git version and signing-helper support are evidence gates, not guessed constants.

## Task 1: Prove the installed Git's publication primitives

**Files:** Create `tests/git_primitives.rs`; modify `docs/implementation.md` only to record measured support.

1. Write disposable-repository fixtures using the installed Git to test NUL-safe status/index/diff output, private `GIT_INDEX_FILE`, `commit-tree` signing behavior, and `git update-ref --stdin -z` with symbolic HEAD verification in no-dereference mode plus expected-old-value branch update.
2. Run `cargo test --locked --test git_primitives`; expect the first fixtures to fail until the exact invocation and output parser are known. Record the tested Git version and rejected variants.
3. If atomic symbolic verification is unavailable on the minimum supported Git, keep 3B disabled; do not replace it with check-then-unconditional-update. Commit `test: establish Git publication primitives`.

## Task 2: Add a bounded Git adapter and repository scope checks

**Files:** Create `src/git/mod.rs`, `src/git/command.rs`, `src/git/repository.rs`; modify `src/lib.rs`; test `tests/git_scope.rs`.

1. Add red fixtures for normal repo, bare repo, detached/unborn HEAD, linked worktree, merge/rebase/sequencer state, unmerged entries, sparse/split index, submodule changes, configured hooks, and unusual path bytes.
2. Implement typed Git command results with an explicit executable/config/environment policy, timeout and output caps, and NUL-safe parsers. Resolve `--git-path` locations and effective hook/config values through Git. No arbitrary Lua-provided Git arguments reach the adapter.
3. Return a typed supported/plan-only/unsupported scope result with a clear reason. Run `cargo test --locked --test git_scope`; expect pass. Commit `feat: inspect supported Git repository scope`.

## Task 3: Prepare and approve read-only staged inspection

**Files:** Create `src/git/snapshot.rs`; modify `src/permissions/action.rs`, `src/permissions/policy.rs`, `src/terminal.rs`; test `tests/git_snapshot.rs`, `tests/permissions.rs`.

1. Add failing fixtures proving denial does not read staged data, the preview names repository and staged-data scope, a stale/changed repository fails, and index/working-tree bytes are identical before and after inspection.
2. Add one specialized `PreparedAction::GitSnapshot` that binds repository identity and a fixed internal read set. Hold the appropriate index lock only for a short consistent snapshot; never while awaiting approval or AI.
3. Capture symbolic branch, HEAD H0, staged tree I, original index bytes/fingerprint, relevant index flags, effective Git policy, and bounded staged data. Release locks before returning. Run focused tests and commit `feat: capture approved staged snapshots`.

## Task 4: Assign opaque staged-change IDs

**Files:** Create `src/git/change.rs`; modify `src/git/snapshot.rs`; test `tests/git_changes.rs`.

1. Add red fixtures for partial staging, add/delete, rename endpoints, executable-mode change, paths with spaces/newlines/non-UTF-8, and independent file groups.
2. Build change units from staged object IDs/modes and exact path bytes, coupling rename endpoints and related transitions. Assign bounded opaque IDs; Lua/model views carry IDs and redacted/bounded diff data, not operation paths.
3. Fail explicitly when a path cannot cross the Lua UTF-8 boundary without loss. Run `cargo test --locked --test git_changes`; expect pass. Commit `feat: identify staged change units`.

## Task 5: Validate model grouping and messages in Rust

**Files:** Create `src/git/plan.rs`; modify `src/schema.rs` only if the supported subset needs a documented extension; test `tests/git_plan.rs`.

1. Add red matrices for complete coverage, duplicate/missing/unknown IDs, oversized or empty groups, invalid order, invalid Conventional Commit message, and model-provided raw path or Git command. Intermediate-tree validation follows Task 6.
2. Accept only ordered groups of opaque IDs plus messages and bounded rationale. Resolve project-specific message rules only from explicit validated configuration or a supported rule source. Compute a stable plan ID/digest binding repository, branch, H0, I, index fingerprint, groups, messages, and execution policy.
3. Show the full validated plan before any publication. Run `cargo test --locked --test git_plan`; expect pass. Commit `feat: validate grouped commit plans`.

## Task 6: Construct candidate trees with private indexes

**Files:** Create `src/git/tree.rs`; test `tests/git_preservation.rs`.

1. Add red disposable fixtures for partial staging, add/delete/rename/mode changes, unusual paths, two independent groups, invalid intermediate trees, and user edits made after snapshot. Save original index bytes, working-tree bytes, H0, and staged tree I.
2. For ordered groups G1..Gn, construct T1..Tn from H0 using the snapshot's staged object IDs/modes and private `GIT_INDEX_FILE` files. Require Tn = I and validate every intermediate tree. Never reread working-tree files to populate a group.
3. Assert original index bytes and unstaged changes remain unchanged before and after candidate construction. Run `cargo test --locked --test git_preservation`; expect pass. Commit `feat: construct grouped trees privately`.

## Task 7: Ship `koru commit` in plan-only mode (3A)

**Files:** Create `examples/commands/commit.lua`; modify `src/lua/bridge.rs`, `src/lua/command.rs`, `src/main.rs`, `README.md`, `docs/lua-api.md`; test `tests/commit_cli.rs`.

1. Add failing CLI fixtures for approved snapshot, noninteractive denial, missing model/key, valid grouped proposal, invalid proposal, unsupported repo, and plan display with zero branch/index/worktree mutations.
2. Expose a narrow Rust Git snapshot/plan host API to Lua through the existing owner bridge. The Lua reference command requests the snapshot, obtains model grouping, returns IDs/messages, and asks Rust to validate/render. It does not execute `git add`, `git commit`, `git reset`, stash, or shell commands.
3. Run `cargo test --locked --test commit_cli --test git_snapshot --test git_plan` and `just check`. Document plan-only status visibly. Commit `feat: add plan-only koru commit`.

## Task 8: Add durable journal and reconciliation

**Files:** Create `src/state/journal.rs`, `src/git/recovery.rs`; modify `src/paths.rs`, `src/persist.rs`, `src/lib.rs`; test `tests/git_recovery.rs`.

1. Add failure-injection fixtures before/after journal flush, candidate creation, ref transaction, and completion recording. Include corrupt schema, wrong repository, changed index/branch, missing candidate, and concurrent Koru operation.
2. Persist a versioned, restricted-access journal under `$XDG_STATE_HOME/koru`; lock scoped writes, validate, write a same-directory temporary file, flush file and directory, then atomically replace. Reject platforms without required durability semantics for automatic publication.
3. Reconcile pending publication: branch at candidate means completed, at expected parent means not published, any other value is conflict. No ambiguous failure triggers an unguarded retry. Run `cargo test --locked --test git_recovery`; expect pass. Commit `feat: persist and reconcile Git operations`.

## Task 9: Guard candidate publication and policy

**Files:** Create `src/git/publish.rs`, `src/git/policy.rs`; modify `src/permissions/action.rs`, `src/permissions/policy.rs`; test `tests/git_publication.rs`, `tests/git_concurrency.rs`.

1. Add red fixtures for signing required/signer failure, applicable hook detection, branch switch/movement, concurrent staging, index-lock contention, changed Git config/executable, and cancellation between groups. Assert stale plans never overwrite index/branch state.
2. Prepare each group as one specialized broker action enumerating private-index/object/journal writes and the exact expected branch transition. Obtain one approval per group outside Git locks; use the plan digest and expected parent in its identity.
3. Create commit objects with explicit parent/tree/message/author/committer/signing policy. Journal and flush before publication. Under short Koru/index locks, revalidate index/policy/branch and use the proven Git reference transaction. Release the index lock without replacing the user's index, then durably record completion.
4. Reject enabled applicable hooks until a tested integration exists; never silently bypass required signing. Run `cargo test --locked --test git_publication --test git_concurrency`; expect pass. Commit `feat: publish guarded commit groups`.

## Task 10: Expose recovery and finish execution (3B)

**Files:** Modify `src/main.rs`, `src/git/recovery.rs`, `examples/commands/commit.lua`, `README.md`, `docs/implementation.md`; test `tests/commit_cli.rs`, `tests/git_recovery.rs`.

1. Add red CLI fixtures for `koru recover <operation-id>` inspection, safe resumption after a published prefix, renewed approvals for remaining groups, and conflict without mutation. Verify candidate parent/tree/message against the plan before resumption.
2. Enable automatic grouped publication only when Tasks 1–9 pass on the supported platform. Report completed/remaining groups and recovery ID on failure; retain published commits and staged remainder. Do not reset, rewrite history, or auto-delete uncertain journals.
3. Run `cargo test --locked --test commit_cli --test git_recovery --test git_preservation --test git_concurrency`; expect pass. Commit `feat: resume guarded grouped commits`.

## Task 11: Final gates and documentation

**Files:** Modify `README.md`, `docs/implementation.md`, `docs/DESIGN.md` only for resolved decisions; focused tests above.

1. Run `just check`, record actual test count, installed Git version, supported repository matrix, and measured `cargo build --locked --release` size. Run all failure-injection and unusual-path fixtures separately.
2. Verify the mandatory preservation property: original index bytes stay unchanged; after a published prefix, HEAD-to-I is the staged remainder and I-to-working-tree is the unstaged remainder; at completion HEAD tree equals I. Verify no duplicate publication after every injected crash point.
3. Document plan-only fallback, hook/signing restrictions, recovery procedure, and explicit unsupported cases. Keep any live-provider smoke test opt-in. Commit `docs: record grouped commit release evidence`.

## Release gates

**3A:** A validated plan covers every staged change exactly once and makes no branch/index/worktree mutation. Denial performs no read; invalid model output cannot become an operation.

**3B:** Candidate trees preserve staged/unstaged content; each group has its own exact approval; signing/hooks policy is honored; the verified reference transaction prevents stale publication; durable reconciliation resolves every injected crash point without lost work or duplicate commits. If any gate fails, `koru commit` remains plan-only.
