# Koru

Koru is a planned Rust CLI for repeatable AI-assisted workflows written in Lua. The intended v1 behavior and its release gates are specified in [docs/DESIGN.md](docs/DESIGN.md).

This repository contains four foundation increments. Koru discovers installed command filenames, captures an immutable bounded command/module source bundle, evaluates a script's declaration in a restricted Lua VM under instruction, memory, and wall-clock limits, and provides a single-VM bridge that suspends a workflow coroutine on `koru.ai` calls while bounded channels carry tool calls to a service. It also has a provider boundary with a versioned configuration file, environment credentials, a bounded service catalog, and capability preflight. No network transport is wired, and Koru does not execute a process, write files, or create Git commits. The `koru <command>` CLI still returns an explicit unsupported-capability error until a provider transport and executor exist.

## Build and try it

Use Rust 1.88 or newer (the current checks run on 1.98):

```sh
cargo build --locked
cargo run --locked --
cargo run --locked -- --inspect hello
cargo run --locked -- check hello
just check
```

Install command sources under `$XDG_CONFIG_HOME/koru/commands/` or `~/.config/koru/commands/`. A top-level `hello.lua` is discovered as `hello`. Modules live under `commands/lib/` and must be declared in leading comments:

```lua
-- koru-module: shared.format
return {
  api_version = 1,
  description = "Example",
  run = function(koru, args) end,
}
```

`koru --inspect NAME` reports the SHA-256 source-bundle digest and module count without evaluating Lua. `koru check [NAME]` evaluates and validates declarations, including tool schemas compiled to a documented JSON Schema subset, under the bounded loader; without a name it checks every discovered command. It grants no permissions and performs no effects. `koru model [service[/model]]`, `koru model update <service>|--all`, and `koru variant [name]` inspect and persist the selection. See [docs/declaration.md](docs/declaration.md) for the API version 1 field reference, [docs/lua-api.md](docs/lua-api.md) for the `koru.json`/`koru.ai` runtime API, and [docs/providers.md](docs/providers.md) for configuration, credentials, catalogs, and capabilities.

Implementation status, current limits, and the next release gates are in [docs/implementation.md](docs/implementation.md). Plans are under [docs/plans/](docs/plans/).
