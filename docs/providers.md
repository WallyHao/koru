# Provider boundary, configuration, and selection

Status: `config.toml`, environment credentials, the bounded service catalog and its
cache, adapter capability preflight, the blocking HTTP transport, the DeepSeek and
OpenCode adapters, and the `koru model`/`koru variant` commands are implemented. The
adapters are exercised end to end against fixture transports; there is no opt-in live
smoke test yet. `koru <command>` runs Lua workflows with the selected adapter. The
Linux shell action and staged Git plan are available through their brokered APIs;
Lua file effects and automatic commit publication are not implemented.

## Configuration

`$XDG_CONFIG_HOME/koru/config.toml` (falling back to `~/.config/koru/config.toml`)
carries the selection. v1 is a strict flat subset:

```toml
schema_version = 1
provider = "deepseek"
model = "deepseek-chat"
variant = "high"
```

- Only `schema_version` (integer, required) and the optional string keys `provider`,
  `model`, and `variant` are accepted. Blank lines and full-line `#` comments are
  allowed; unknown keys, duplicate keys, malformed lines, bare values, unsupported
  escapes, and non-UTF-8 fail with a line-qualified error.
- A missing file means no selection. An unsupported `schema_version` fails rather than
  being ignored. A variant requires a selected provider and model.
- Writes are atomic (same-directory temporary file plus rename) and read-modify-write
  holds a shared lock, so concurrent selection cannot lose an update. A lock older
  than 60 seconds is treated as abandoned and replaced; a fresh lock that cannot be
  taken fails with actionable guidance.

## Credentials

`DEEPSEEK_API_KEY` and `OPENCODE_API_KEY` are read from the environment. They are never
written to `config.toml`, never exposed to Lua, and are removed from error and
diagnostic text. A missing credential for a chosen provider fails with a `validation`
error naming the variable; there is no silent fallback.

## Transport

Adapters talk to providers through a blocking `Transport` boundary:

- The production transport is `ureq` over rustls with pinned webpki roots. There is no
  async runtime.
- Requests and responses are bounded; connect and read timeouts apply; redirects are
  disabled so credentials can never be forwarded to another host; the user agent is
  `koru/<version>`.
- Any HTTP status is returned as a response; only transport-level failures (DNS,
  connect, TLS, protocol, size, redirect) are classified. Provider errors are redacted
  before they reach the user.
- A deterministic fixture transport records requests and replays scripted responses or
  failures for tests.

## JSON codec

Protocol payloads and cache files use a Koru-owned bounded JSON codec: a strict parser
and a canonical emitter over `JsonValue`. It rejects trailing data, duplicate keys,
unescaped controls, invalid escapes, lone surrogates, non-finite numbers, and anything
beyond the depth, element, or byte limits. Integral floats keep an explicit `.0` so a
round trip restores the same variant.

## Services, adapters, and the catalog

Three service identities are distinct: `deepseek`, `opencode`, and `opencode-go`. Each
has its own credentials, metadata, and cache. DeepSeek and the OpenCode surfaces use one
Chat Completions mapping; OpenCode Go additionally sends a stable
`x-opencode-session` on every request, including tool-loop turns and retries.

Catalog parsing consumes a bounded JSON document with a `data` array. Unknown provider
fields are ignored; wrong types or oversized values fail with `validation`. A missing
capability field is **unknown** and is never treated as supported.

`koru model update` fetches live metadata through the adapter and caches it under
`$XDG_CACHE_HOME/koru/catalog/<service>.json`. The cache is disposable: corrupt or
unsupported data is ignored and refetched, and writes are atomic under the shared lock.
Refreshing one service replaces only that service and reuses its previous entries when
the fetch fails. Selection is checked against the best available cache: a model absent
from a cached catalog is reported with a pointer to `koru model update`.

## Capability preflight

Each adapter reports `ServiceCapabilities`: tool support, structured-output support, the
schema keywords it accepts, declared variants, and session use. Before any
`koru.ai.run` dispatches, the bridge checks the request: tools without tool support or a
schema using an unsupported keyword fail with `unsupported_capability`, before the
service thread starts and before any model-request budget is charged.

## Retries

A model turn is attempted at most three times. Only explicitly transient failures are
retried: connect, timeout, protocol, and the HTTP statuses 408, 429, 500, 502, 503, and
504. Each retry reserves one model request from the shared budget through the VM owner,
so a retry storm stops at the budget and the whole-command deadline. A bounded
`Retry-After` is honored. A request is never replayed after a tool call has executed.

## Commands

- `koru model` prints the current selection and the implemented services.
- `koru model <service>` selects a provider and clears the model and variant.
- `koru model <service>/<model>` selects a provider and model, checked against the
  cache; an invalid variant is cleared with a notice.
- `koru model update <service>` and `koru model update --all` fetch and cache live
  metadata, report a removed selected model, and require the service credential.
- `koru variant` lists the variants valid for the current selection; `koru variant
  <name>` sets one. Without a selected model it fails with `validation`.

Commands are noninteractive: they never prompt, and a selected model reported as removed
or unavailable is reported rather than silently substituted.

## Not implemented

An opt-in live provider smoke test, streaming, provider-native structured output,
interactive selection, and Lua file effects. Shell effects and the plan-only Git API
are documented in `docs/lua-api.md`; automatic commit publication remains disabled.
