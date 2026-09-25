# Koru

Koru is a Rust CLI for repeatable AI-assisted workflows written in Lua. The intended v1 behavior and its release gates are specified in [docs/DESIGN.md](docs/DESIGN.md).

Koru discovers installed Lua commands, captures and validates their source, and runs them in a bounded VM. Commands can call the selected DeepSeek, OpenCode Zen, or OpenCode Go model through `koru.ai.ask`, `koru.ai.run`, and schema-validated `koru.ai.ask_json`. The installed `shell` command can propose a script and run it only after terminal approval. Shell execution and staged Git snapshotting are Linux-only. The `commit` command groups staged changes into a validated, reviewable plan; it does not create commits.

## Build and try it

Use Rust 1.88 or newer (the current checks run on 1.98):

```sh
cargo build --locked
cargo run --locked --
cargo run --locked -- --inspect hello
cargo run --locked -- check hello
just check
```

For a runnable AI command, install the included example and select a model:

```sh
mkdir -p ~/.config/koru/commands
cp examples/commands/ask.lua ~/.config/koru/commands/ask.lua
cp examples/commands/shell.lua ~/.config/koru/commands/shell.lua
cp examples/commands/commit.lua ~/.config/koru/commands/commit.lua
export DEEPSEEK_API_KEY=your_key
cargo run --locked -- model deepseek/deepseek-chat
cargo run --locked -- check ask
cargo run --locked -- ask "Explain this function"
cargo run --locked -- check shell
cargo run --locked -- shell "list the largest files in this directory"
cargo run --locked -- check commit
cargo run --locked -- commit
```

Set `XDG_CONFIG_HOME` if your command directory is elsewhere. `koru shell` accepts one task argument or prompts for it in an interactive terminal. Each proposed command requires a separate exact-action approval; redirected sessions cannot approve shell actions. `koru commit` requests approval to read the staged diff, then returns a plan with grouped changes and Conventional Commit subjects. Git must be installed on `PATH`; the command does not alter the index, worktree, or Git refs. Provider credentials stay in the environment and are not available to Lua or child processes.

Install command sources under `$XDG_CONFIG_HOME/koru/commands/` or `~/.config/koru/commands/`. A top-level `hello.lua` is discovered as `hello`. Modules live under `commands/lib/` and must be declared in leading comments:

```lua
-- koru-module: shared.format
return {
  api_version = 1,
  description = "Example",
  run = function(koru, args) end,
}
```

`koru --inspect NAME` reports the SHA-256 source-bundle digest and module count without evaluating Lua. `koru check [NAME]` evaluates and validates declarations, including tool schemas compiled to a documented JSON Schema subset, under the bounded loader; without a name it checks every discovered command. It grants no permissions and performs no effects. `koru model [service[/model]]` and `koru variant [name]` select and persist a model, and `koru model update <service>|--all` fetches and caches live provider metadata. See [docs/declaration.md](docs/declaration.md) for the API version 1 field reference, [docs/lua-api.md](docs/lua-api.md) for the `koru.json`/`koru.ai` runtime API, and [docs/providers.md](docs/providers.md) for configuration, credentials, catalogs, capabilities, transport, and retries.

Implementation status, current limits, and the next release gates are in [docs/implementation.md](docs/implementation.md). Plans are under [docs/plans/](docs/plans/).
