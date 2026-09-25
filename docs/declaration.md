# Command declaration (Koru API version 1)

Status: implemented for bounded declaration loading and `koru check`. This is the
authoritative field reference for the declaration returned by a command script.

A command is one Lua file under `$XDG_CONFIG_HOME/koru/commands/`. The file is
evaluated once in a restricted VM and must return one table:

```lua
return {
  api_version = 1,
  description = "Summarize a repository change",
  arguments = {
    { name = "task", type = "string", required = true, help = "What to do" },
    { name = "count", type = "integer", default = 1, min = 1, max = 10 },
    { name = "mode", type = "enum", values = { "fast", "careful" }, default = "fast" },
  },
  capabilities = { direct_processes = false },
  run = function(koru, args) end,
}
```

## Fields

| Field | Required | Type | Notes |
| --- | --- | --- | --- |
| `api_version` | yes | integer | Must equal `1`; other values fail as unsupported. |
| `description` | yes | string | 1 to 1024 bytes, no control characters. |
| `arguments` | no | array | At most 32 entries, contiguous from index 1. |
| `capabilities` | no | table | Only `direct_processes` is known. A request, never a grant. |
| `tools` | no | array | At most 32 tool declarations; see below. |
| `run` | yes | function | Validated as a function; `koru check` never calls it. |

Unknown fields are validation errors. Requested `exemptions` remain rejected
with `unsupported_capability` until the durable exemption store exists, so a
passing check never implies support for them.

## Tool declarations

Each `tools` entry is a table:

| Field | Required | Type | Notes |
| --- | --- | --- | --- |
| `name` | yes | string | `[a-z][a-z0-9_]*`, at most 64 bytes, unique. |
| `description` | yes | string | 1 to 1024 bytes, no control characters. |
| `parameters` | yes | object | JSON schema for the arguments; must describe an object. |
| `result` | no | object | Optional JSON schema for the callback result. |
| `run` | yes | function | Callback invoked with JSON arguments; returns JSON. |

Tool names are checked against the declaration; `koru.ai.run` may offer only a
declared subset. Callbacks are never invoked by `koru check`. `parameters` and
`result` are compiled to the documented schema subset; unsupported keywords fail
validation. See `docs/lua-api.md` for the subset and the runtime
`koru.json`/`koru.ai` behavior.

Structured workflow results use `koru.ai.ask_json` with an explicit schema and
`prompt_validate` mode. The schema is compiled with the same Koru-owned subset
as tool schemas; it does not grant a provider capability or an effect.

## Argument entries

| Field | Required | Type | Notes |
| --- | --- | --- | --- |
| `name` | yes | string | `[a-z][a-z0-9_]*`, at most 64 bytes, unique. |
| `type` | yes | string | `string`, `integer`, `number`, `boolean`, or `enum`. |
| `required` | no | boolean | Defaults to `false`; cannot combine with `default`. |
| `default` | no | literal | Must match the declared type. |
| `help` | no | string | At most 1024 bytes. |
| `min`, `max` | no | number | Numeric types only; finite and ordered. |
| `max_len` | no | integer | String type only. |
| `values` | enum only | array | 1 to 64 unique strings. |

Integer defaults must stay within the portable JSON range of `+/- (2^53 - 1)`;
larger identifiers use strings.

CLI workflow arguments are positional in declaration order. They are parsed and
validated before a provider is constructed: `true` and `false` are the only
boolean spellings, numeric values must be finite and within declared bounds,
and omitted optional arguments use their default or JSON null. A missing
required argument fails with `validation` in noninteractive mode.

## Environment and module loading

- The VM is built from an explicit allowlist: `table`, `string`, `utf8`, `math`.
  `io`, `os`, `package`, `debug`, `coroutine`, `load`, `loadfile`, `dofile`,
  `print`, `warn`, and `collectgarbage` are removed. The `koru` host table is not
  defined during declaration evaluation.
- `require` is a Koru host function. It resolves only names present in the
  captured source bundle, caches each module once per run, and never reads the
  filesystem or loads bytecode. A module declares its dependencies with leading
  `-- koru-module: module.name` directives; undeclared or cyclic loads fail.
- Modules and the entry are subject to the same instruction, memory, and
  wall-clock ledger as the command. Exhausted, cancelled, or timed-out contexts
  are terminal even if a script catches the abort with `pcall`.

## Errors

Fallible calls return a value on success or `nil, error` on failure. The CLI
prints a stable code per failure: `validation`, `budget_exhausted`,
`permission_denied`, `unsupported_capability`, `state_conflict`, `cancelled`,
`timeout`, or `io`. Declaration errors are bounded and control characters are
escaped before display.

## Reference

`koru check [command]` validates declarations: syntax and evaluation under the
bounded loader, the field schema above, and API version support. It reads only
command and module sources and grants no permission. See `docs/DESIGN.md` for the
full v1 lifecycle and `docs/implementation.md` for what is not implemented yet.
