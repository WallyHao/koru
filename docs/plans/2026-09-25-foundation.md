# Koru Foundation Implementation Plan

> **For Codex:** Use the Executing Plans skill to implement this plan task-by-task.

**Goal:** Deliver the first independently verifiable implementation increment of DESIGN.md: immutable source loading, execution budgets, and permission contracts.

**Architecture:** A single Rust crate owns contracts and pure validation. Filesystem discovery and source capture are adapters; policy and budget state remain Rust-owned. No model, subprocess, workflow evaluation, or Git publication is enabled in this increment.

**Tech Stack:** Rust 2024, clap for CLI parsing, sha2 for source identity, thiserror for contextual failures, tempfile for isolated integration fixtures. Cargo.lock pins the dependency graph. Lua/Rig selection belongs to the next measured prototype.

---

## Task 1: Source boundary and CLI

Files: Cargo.toml, src/lib.rs, src/error.rs, src/paths.rs, src/source/{mod,names,discovery,bundle}.rs, src/main.rs, tests/source.rs, tests/cli.rs.

1. Create the single-crate manifest, dependency lockfile, task recipes, and ignore rules.
2. Write filesystem boundary fixtures before implementation: discovery without evaluation, reserved collisions, transitive capture, cycles, traversal, bytecode, source limits, digest sensitivity, immutable captured bytes, symlink rejection.
3. Run `cargo test --test source --test cli`; verify the initial missing implementation failure.
4. Implement strict names, bounded regular-file capture, leading module directives, transitive DAG loading, and domain-separated SHA-256 framing.
5. Implement `koru` discovery and `koru --inspect <command>` metadata output. Workflow execution and `check` explicitly fail as not implemented; never imply source inspection validates Lua syntax.
6. Run focused tests and `cargo fmt`.

## Task 2: Shared terminal execution state

Files: src/runtime/{mod,budget}.rs, tests/runtime.rs.

1. Write tests for atomic multi-resource reservation, hard ceilings, shared clones, irreversible exhaustion/cancellation, and expired deadlines.
2. Run `cargo test --test runtime` and confirm missing implementation failures.
3. Implement a typed immutable limits snapshot with hard ceilings and a shared, synchronized ledger; reserve before dispatch. Capture command/source identity at context creation.
4. Re-run focused tests; check that terminal states never allow further reservation.

## Task 3: Prepared operations and user-owned policy

Files: src/permissions/{mod,action,policy}.rs, tests/permissions.rs.

1. Write tests for default denial, exact process grants, source/policy/argument/environment identity changes, shell non-exemption, action/context binding, one-use authorization, and cancellation after approval.
2. Run `cargo test --test permissions` and confirm missing implementation failures.
3. Implement immutable prepared process/shell values and broker-owned one-use authorization. Requests have no conversion into grants; only host-owned policy can authorize. No OS execution adapter is enabled yet.
4. Verify focused tests and public API documentation.

## Completion checks

Run `just check` (format, clippy with warnings denied, all tests, Rustdoc).
Run CLI smoke tests against isolated fixture directories; measure `cargo build --release --locked` output size.
Record actual outcomes and known limitations in docs/implementation.md. Preserve the original design and backup. Initialize local version history and commit only this increment after checks pass; do not publish anything.

## Deferred release gates

Bounded Lua declaration validation and JSON conversion; single-VM async bridge; real process/file identity revalidation and cleanup; persistent grants/config locks; provider transports; Git planning/publication/recovery. None is claimed by this foundation increment. Revisit exact public contracts when their first adapters are added, before declaring API v1 stable.
