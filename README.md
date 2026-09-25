# Koru

Koru is a planned Rust CLI for repeatable AI-assisted workflows written in Lua. The intended v1 behavior and its release gates are specified in [docs/DESIGN.md](docs/DESIGN.md).

This repository currently contains the first foundation increment. It can discover installed command filenames and inspect an immutable, bounded command/module source bundle. It does not evaluate Lua, call a model, execute a process, or create Git commits. Commands and `koru check` return an explicit unsupported-capability error until their runtimes are implemented.

## Build and try the foundation

Use Rust 1.98 or newer (the version used for the current checks):

```sh
cargo build --locked
cargo run --locked --
cargo run --locked -- --inspect hello
just check
```

Install command sources under `$XDG_CONFIG_HOME/koru/commands/` or `~/.config/koru/commands/`. A top-level `hello.lua` is discovered as `hello`. Modules live under `commands/lib/` and must be declared in leading comments:

```lua
-- koru-module: shared.format
return { api_version = 1, description = "Example", run = function(koru, args) end }
```

The declaration above is illustrative; the current binary only captures its bytes. Inspect reports the SHA-256 source-bundle digest and module count. It does not validate declaration syntax.

Implementation status, current limits, and the next release gates are in [docs/implementation.md](docs/implementation.md). The first increment's task plan is in [docs/plans/2026-09-25-foundation.md](docs/plans/2026-09-25-foundation.md).
