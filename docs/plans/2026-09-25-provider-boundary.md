# Koru Provider Boundary, Configuration, and Capability Preflight Plan

**Goal:** Deliver the offline-testable core of the DESIGN.md `Providers` gate and the
user-facing selection surface: a Koru-owned service catalog and capability model, a
capability preflight that stops unsupported runs before any model request, typed
`config.toml` persistence, environment credentials with redaction, and the
`koru model` / `koru variant` / `koru model update` commands exercised against a
fixture metadata source. No real network transport, no terminal approval, and no
`koru shell` in this increment.

**Architecture:** Provider/service identity, catalog metadata, adapter capability, and
workflow requirements stay three separate Rust-owned concepts. A `CatalogSource`
boundary returns bounded `JsonValue` documents; a strict parser turns them into
validated catalog entries. `AiService` gains a capability descriptor, and the single-VM
bridge runs a preflight before dispatch. Configuration is a flat, versioned file read and
written under a scoped lock with atomic replacement. Credentials live only in the
environment and are redacted at error boundaries.

**Tech Stack:** unchanged: Rust 2024, `mlua`, `sha2`, `thiserror`, `clap`, `tempfile`
(dev). This increment adds no HTTP client, no async runtime, and no serialization
framework; the bounded catalog parser consumes the existing `JsonValue` model.

---

## Scope

In scope:

1. Versioned `config.toml` model with a strict flat parser/renderer, atomic replacement,
   and a scoped writer lock so concurrent selection does not lose updates.
2. Environment credentials (`DEEPSEEK_API_KEY`, `OPENCODE_API_KEY`) with a redaction
   boundary; credentials never enter `config.toml` and never reach Lua.
3. Service identities and a bounded catalog model for DeepSeek, OpenCode Zen, and
   OpenCode Go, with a `CatalogSource` trait and a deterministic fixture source.
4. Adapter capability descriptors and a bridge preflight: unsupported tools, schemas, or
   structured output fail with `unsupported_capability` before any request.
5. `koru model`, `koru model <service>[/<model>]`, `koru variant`, and
   `koru model update <service>|--all`, noninteractive only.
6. Error normalization and redaction for catalog and credential failures.
7. Tests for every bullet above, including concurrent config writers and the preflight
   matrix.

Out of scope (later increments): real HTTP transport and live endpoints, provider
protocol payloads, streaming, retry classification, on-disk catalog cache persistence,
interactive terminal selection, `koru shell`, tool/process effects, durable grants,
and Git.

---

## Specification

The following are normative. `MUST`/`MUST NOT` are hard requirements; `SHOULD` allows a
recorded justification.

### S1. Configuration file

- The file is `$XDG_CONFIG_HOME/koru/config.toml` (via `UserPaths`). It carries
  `schema_version = 1`.
- v1 is a strict flat TOML subset with only these keys: `schema_version` (integer,
  required), `provider` (string), `model` (string), `variant` (string). Each string key
  is optional.
- Parsing MUST accept blank lines and `#` comments, and double-quoted basic strings with
  the escapes `\"`, `\\`, `\n`, `\t`. It MUST reject unknown keys, duplicate keys,
  malformed lines, bare non-integer values, out-of-range `schema_version`, and
  non-UTF-8 input with a `Validation` error naming the line.
- A missing file yields the default (no selection). An unsupported `schema_version`
  fails explicitly; it is never silently ignored.
- Writing MUST use a same-directory temporary file followed by an atomic rename, and
  MUST NOT leave a partial file on failure.
- Read-modify-write MUST hold a scoped lock so two concurrent writers do not lose an
  update. The lock is an exclusively created lock file with a bounded wait; contention
  beyond the bound fails with an actionable error. Crash-safe stale-lock recovery is
  explicitly deferred to the persistence gate and MUST be documented.

### S2. Credentials

- `DEEPSEEK_API_KEY` and `OPENCODE_API_KEY` are read from the environment through an
  injected accessor so tests never mutate process globals.
- Credentials MUST NOT be written to `config.toml`, logged, or exposed to the Lua
  environment. Errors and diagnostics MUST pass through a redaction helper that removes
  known credential values.
- A missing credential for a service produces an actionable `Validation` error naming
  the environment variable; it does not silently fall back.

### S3. Service catalog

- `ServiceId` distinguishes `DeepSeek`, `OpenCode`, and `OpenCodeGo`; their identities,
  metadata, and (later) caches stay separate. Service prefixes are part of model
  identity.
- `CatalogEntry` carries at least: model id, service, display name, optional context
  length, optional tool support, optional structured-output support, declared variants,
  source, and fetch time. Missing metadata stays explicitly unknown and MUST NOT be
  treated as supported.
- Parsing consumes a bounded `JsonValue` document under `JsonLimits`: unknown top-level
  shape, missing required fields, oversized values, and wrong types fail with a bounded
  `Validation` or `ProviderFailure` error.
- `Catalog::replace_service(service, entries)` replaces only that service's entries.
  A fetch or parse failure retains the previous entries for that service and reports the
  affected service.
- A `CatalogSource` trait returns a bounded document per service. A fixture source backs
  tests; the production source returns `unsupported_capability` until a real transport
  exists. On-disk catalog caching is deferred to the persistence gate.

### S4. Adapter capabilities and preflight

- `ServiceCapabilities` describes adapter-owned facts: provider identity, whether tools
  are supported, whether structured output is supported, the supported schema keywords
  (or the full Koru subset), declared variants, and whether session headers are used.
- `AiService::capabilities(&self) -> ServiceCapabilities` is immutable for a run.
- Before dispatching any `koru.ai.run`, the bridge MUST preflight the request against the
  service capabilities:
  - tools requested but `tools` unsupported fails with `unsupported_capability`;
  - a requested tool schema using a keyword outside the adapter's supported set fails
    with `unsupported_capability`;
  - an unknown capability is treated as unsupported.
- Preflight MUST fail before the service thread is spawned and before any model request
  budget is charged. Catalog-level eligibility (metadata says the model lacks tools or
  structured output) is checked at selection time; a runtime requirement absent from
  metadata is not treated as supported.
- The selected model identity remains visible in `AiResult.model`.

### S5. Selection commands

- `koru model` prints the current provider/model/variant and the implemented services.
- `koru model <service>` and `koru model <service>/<model>` set and persist the
  selection. The service must be an implemented identity; a malformed identity fails
  with `Validation`.
- `koru model update <service>` and `koru model update --all` fetch through the catalog
  source and report entries or a normalized failure. In this increment the production
  source reports `unsupported_capability`; the fixture source is used in tests.
- `koru variant` lists the variants valid for the current selection; `koru variant
  <name>` sets and persists one. An invalid variant or missing selection fails with
  `Validation`.
- Changing the model and clearing a variant that is invalid for the new service/model
  MUST print a notice rather than silently keeping it.
- Commands are noninteractive: they never prompt and never wait for input. A selected
  model reported as removed or unavailable is reported, never silently substituted.

### S6. Errors

- Catalog parse failures use `Validation`; transport/metadata failures use
  `ProviderFailure`; missing credentials use `Validation`; unsupported adapter features
  use `UnsupportedCapability`. All messages are bounded and redacted.
- Terminal context semantics and existing bridge error codes are unchanged.

---

## Task 1: Configuration module

Files: `src/config.rs`, `src/lib.rs`, `tests/config.rs`.

1. Define the typed `Config` model and defaults; add `UserPaths::config_file()`.
2. Implement the strict flat parser/renderer from S1 with line-qualified errors.
3. Implement atomic save and the scoped lock with bounded contention.
4. Tests: round-trip, comments/escapes, unknown/duplicate/malformed rejection, bad
   `schema_version`, missing file default, atomic replace, and two writers not losing an
   update.

## Task 2: Credentials and redaction

Files: `src/credentials.rs`, `src/error.rs`, `src/lib.rs`, `tests/credentials.rs`.

1. Define `Credentials` with environment names per service and an injected accessor.
2. Add a bounded redaction helper and route catalog/credential error text through it.
3. Tests: present/absent variables, no fallback, and redaction of a known secret in a
   longer message.

## Task 3: Catalog model and source

Files: `src/provider/catalog.rs`, `src/provider/mod.rs`, `src/lib.rs`,
`tests/catalog.rs`.

1. Define `ServiceId`, `CatalogEntry`, `Catalog`, and `CatalogSource`.
2. Parse a bounded `JsonValue` document into entries with strict shape checks.
3. Implement the fixture source and the explicit `unsupported_capability` production
   source.
4. Tests: parse accepted/rejected shapes, unknown capability not treated as supported,
   replace-only-requested-service, and fetch/parse failure retaining the previous cache.

## Task 4: Capabilities and bridge preflight

Files: `src/ai/mod.rs`, `src/ai/fake.rs`, `src/lua/bridge.rs`,
`src/lua/bridge/drive.rs`, `tests/bridge.rs`, `tests/preflight.rs`.

1. Add `ServiceCapabilities` and the `AiService::capabilities` method; update the fake.
2. Run preflight in `drive_agent` before spawning the worker and before reserving the
   request budget; map unsupported features to `unsupported_capability`.
3. Tests: tools unsupported, schema keyword unsupported, unknown capability unsupported,
   and a fully supported request still dispatching.

## Task 5: Selection commands

Files: `src/main.rs`, `src/cli.rs` (or the existing CLI module), `tests/cli.rs`.

1. Add `koru model [service[/model]]`, `koru variant [name]`, and `koru model update
   [service|--all]`, noninteractive.
2. Validate and persist through the config module; print the current selection with no
   arguments.
3. Tests: set/clear, missing selection, invalid variant, model change clearing an
   invalid variant with a notice, and `update` against the fixture/inert source.

## Task 6: Documentation and status

Files: `README.md`, `docs/DESIGN.md`, `docs/implementation.md`, `docs/declaration.md` or
`docs/lua-api.md` as needed.

1. Document the config schema and subset, credentials, catalog/capability model,
   preflight behavior, and the selection commands.
2. Update implementation status, measured size, remaining decisions, and the next gate
   (real transport and `koru shell`).

---

## Gate evidence mapping

| DESIGN gate | This increment's evidence |
| --- | --- |
| Providers | Catalog parsing and bounded rejection, capability rejection, variant listing, normalized/redacted failures |
| Persistence (partial) | Config atomic replacement, scoped writer lock without lost updates; stale-lock recovery deferred and documented |
| Lua/AI bridge | Preflight before dispatch; supported requests continue to pass the existing gate |

Git, terminal, and live-provider gates remain unaddressed here by design.

## Completion checks

- `just check` passes (fmt, Clippy `-D warnings`, tests, Rustdoc).
- `cargo build --locked --release` and record the stripped size.
- Confirm no network call, HTTP client, or live endpoint is added.
- Conventional commits authored `wallyhao <wallyhao@qq.com>`, one logical change each,
  subject at most 72 characters.

## Risks and fallbacks

- A hand-written flat parser could grow into a general TOML subset. Mitigation: document
  the v1 subset as frozen and add `toml`/`serde` later only if the schema grows.
- A lock file without stale recovery can block after a crash. Mitigation: bounded wait,
  explicit error, and an entry in the persistence gate to replace it.
- Extending `AiService` ripples through the fake and bridge. Mitigation: keep the new
  method a plain descriptor and update both call sites in one task.
- Production catalog commands are inert until a transport exists. Mitigation: the
  production source fails explicitly with `unsupported_capability` and the commands
  remain fully exercised by the fixture source.

## Follow-on phases (sketch)

- **Phase 2D - Real transport and `koru shell`.** Pin an HTTP client, implement one
  protocol, wire the terminal approval adapter, execute the prepared shell action under
  context-bound limits, and enable the end-to-end command.
- **Phase 3 - Git.** Snapshot, plan, grouped publication, recovery, and concurrency
  behind their dedicated gates.
