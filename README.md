# Koru

Koru is a planned Rust CLI for repeatable AI-assisted workflows written in Lua. The intended v1 behavior and its release gates are specified in [docs/DESIGN.md](docs/DESIGN.md).

This repository contains two foundation increments. Koru discovers installed command filenames, captures an immutable bounded command/module source bundle, and evaluates a script's declaration in a restricted Lua VM under instruction, memory, and wall-clock limits. It does not call a model, execute a process, write files, or create Git commits. Workflow execution returns an explicit unsupported-capability error until its runtime is implemented.

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

`koru --inspect NAME` reports the SHA-256 source-bundle digest and module count without evaluating Lua. `koru check [NAME]` evaluates and validates declarations under the bounded loader; without a name it checks every discovered command. It grants no permissions and performs no effects. See [docs/declaration.md](docs/declaration.md) for the API version 1 field reference.

Implementation status, current limits, and the next release gates are in [docs/implementation.md](docs/implementation.md). Plans are under [docs/plans/](docs/plans/).
