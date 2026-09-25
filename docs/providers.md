# Provider boundary, configuration, and selection

Status: `config.toml`, environment credentials, the bounded service catalog, adapter
capability preflight, and the `koru model`/`koru variant` commands are implemented and
tested against a fixture metadata source. No network transport exists, so
`koru model update` reports `unsupported_capability` and `koru <command>` still cannot
run a workflow.

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
  being ignored.
- A variant requires a selected provider and model. A provider alone is allowed while
  choosing a service.
- Writes create a same-directory temporary file and rename it into place, so a partial
  file is never left behind. Read-modify-write holds a lock file so concurrent selection
  does not lose an update. Crash-safe stale-lock recovery belongs to the persistence
  gate and is not promised yet.

## Credentials

`DEEPSEEK_API_KEY` and `OPENCODE_API_KEY` are read from the environment. They are never
written to `config.toml`, never exposed to Lua, and are removed from error and
diagnostic text. A missing credential for a chosen provider fails with a `validation`
error naming the variable; there is no silent fallback.

## Services and the catalog

Three service identities are distinct: `deepseek`, `opencode`, and `opencode-go`. Each
has its own credentials, metadata, and (later) cache.

Catalog parsing consumes a bounded JSON document with a `data` array. Each model may
carry an `id`, a display name, a context length, a `capabilities` object with `tools`
and `structured_output`, and `variants`. Unknown provider fields are ignored; wrong
types or oversized values fail with `validation`. A missing capability field is
**unknown** and is never treated as supported.

Refreshing one service replaces only that service's entries and leaves its previous
entries intact when the fetch or parse fails. On-disk catalog caching is deferred to the
persistence gate.

## Capability preflight

Each adapter reports `ServiceCapabilities`: whether tools are supported, whether
structured output is supported, which schema keywords it accepts, its variants, and
whether it uses sessions. Before any `koru.ai.run` dispatches, the bridge checks the
request against those capabilities:

- tools requested from a service without tool support fail with
  `unsupported_capability`;
- a requested tool schema that uses a keyword outside the adapter's supported set fails
  with `unsupported_capability`;
- an unknown capability is treated as unsupported.

Preflight runs before the service thread starts and before any model-request budget is
charged.

## Commands

- `koru model` prints the current selection and the implemented services.
- `koru model <service>` selects a provider and clears the model and variant.
- `koru model <service>/<model>` selects a provider and model. A variant that is invalid
  for the new selection is cleared with a notice.
- `koru model update <service>` and `koru model update --all` fetch catalog metadata. In
  this increment the production source reports `unsupported_capability`.
- `koru variant` lists the variants valid for the current selection; `koru variant
  <name>` sets one. Without a selected model it fails with `validation`.

Commands are noninteractive: they never prompt, and a selected model reported as removed
or unavailable is reported rather than silently substituted.

## Not implemented

Real HTTP transport and live endpoints, provider protocol payloads, streaming, retry
classification, on-disk catalog caching, interactive selection, and `koru shell`.
