# Koru

Koru is a Rust command-line application for building repeatable, AI-assisted workflows in Lua. It captures and validates workflow source, runs it in a bounded Lua environment, and gives workflows access to the selected AI provider through a controlled host API.

The current release includes model-assisted questions, Linux shell actions that require explicit terminal approval, and a Linux staged-change planner. The `commit` workflow prepares a validated commit plan; it does not create Git commits. Koru is under active development and is not yet a v1 release.

## Contents

- [Requirements](#requirements)
- [Build and install](#build-and-install)
- [Quick start](#quick-start)
- [Command discovery and validation](#command-discovery-and-validation)
- [CLI reference](#cli-reference)
- [Configure a provider](#configure-a-provider)
- [Use the included workflows](#use-the-included-workflows)
- [Write a workflow](#write-a-workflow)
- [Troubleshooting](#troubleshooting)
- [Security and current limits](#security-and-current-limits)
- [Documentation](#documentation)

## Requirements

- Rust 1.88 or later to build from source.
- A supported AI provider account and its API credential in the environment to run workflows that call a model.
- Linux to use the `shell` and `commit` workflow effects.
- Git on `PATH` to use the `commit` workflow.
- An interactive terminal to approve shell and Git snapshot actions. Approval is unavailable when standard input or standard error is redirected.

## Build and install

From the repository root, build and install the `koru` executable:

```sh
cargo install --path . --locked
koru --version
koru --help
```

To build without installing, use `cargo build --locked`; the executable is written to `target/debug/koru`. To run the project directly during development, use `cargo run --locked -- [ARGS...]`.

Run the repository quality checks with:

```sh
just check
```

This runs formatting checks, Clippy with warnings denied, the test suite, and Rust documentation checks. The `just` command is required for this convenience target; each underlying Cargo command can also be run separately.

## Quick start

The following example installs the included workflows for the current user and configures DeepSeek. Replace the credential placeholder with a key supplied through your approved secret-management process.

```sh
cargo install --path . --locked
export XDG_CONFIG_HOME="${XDG_CONFIG_HOME:-$HOME/.config}"
mkdir -p "$XDG_CONFIG_HOME/koru/commands"
cp examples/commands/ask.lua "$XDG_CONFIG_HOME/koru/commands/ask.lua"
cp examples/commands/shell.lua "$XDG_CONFIG_HOME/koru/commands/shell.lua"
cp examples/commands/commit.lua "$XDG_CONFIG_HOME/koru/commands/commit.lua"

export DEEPSEEK_API_KEY='replace-with-your-key'
koru model update deepseek
koru model deepseek/replace-with-a-model-id

koru check
koru ask "Summarize the purpose of this repository."
```

Replace the model selector with an ID printed by `koru model update deepseek`. The provider, model, and optional variant are stored in `config.toml`. API keys are read from the environment and are not written to that file. See [Configure a provider](#configure-a-provider) for OpenCode selection and catalog commands.

## Command discovery and validation

Koru discovers one workflow per Lua file in:

```text
$XDG_CONFIG_HOME/koru/commands/
```

If `XDG_CONFIG_HOME` is unset, Koru uses `~/.config/koru/commands/`. For example, `commands/ask.lua` is invoked as `koru ask`. A workflow may declare positional arguments and modules; modules are stored below `commands/lib/` and identify themselves with a leading `-- koru-module: module.name` directive.

Use these commands before running a workflow:

```sh
koru                 # List discovered workflow names.
koru check           # Validate every discovered workflow declaration.
koru check ask       # Validate one workflow declaration.
koru --inspect ask   # Show captured source metadata without evaluating Lua.
```

`check` evaluates the declaration in Koru's bounded loader but does not call the workflow's `run` function, contact a provider, or perform effects. `--inspect` reports the captured source digest and module count; it does not check Lua syntax or declaration validity.

## CLI reference

| Command | Purpose |
| --- | --- |
| `koru` | List discovered workflows. |
| `koru <workflow> [ARG...]` | Run a workflow with positional arguments. |
| `koru check [WORKFLOW]` | Validate one workflow or all discovered workflows. |
| `koru --inspect WORKFLOW` | Inspect captured source metadata without evaluating it. |
| `koru model` | Show the current provider, model, and variant selection. |
| `koru model <SERVICE>` | Select a provider and clear the model and variant. |
| `koru model <SERVICE>/<MODEL>` | Select a provider and model. |
| `koru model update <SERVICE>` | Fetch and cache that provider's current model catalog. |
| `koru model update --all` | Refresh the catalogs for all three supported services. |
| `koru variant` | Show variants available for the selected model. |
| `koru variant <NAME>` | Select a supported variant for the selected model. |
| `koru --version` / `koru --help` | Print version or command-line help. |

Supported service identifiers are `deepseek`, `opencode`, and `opencode-go`. Catalog refresh requires network access and the selected provider's credential. Model availability is checked against the best available cached catalog; variants come from the adapter's declared list, which remains provisional. Koru reports an unavailable selection instead of silently changing it.

## Configure a provider

Koru stores provider selection in `$XDG_CONFIG_HOME/koru/config.toml`, or `~/.config/koru/config.toml` when `XDG_CONFIG_HOME` is unset. Koru creates and updates this file through the `model` and `variant` commands. A typical selection looks like:

```toml
schema_version = 1
provider = "deepseek"
model = "deepseek-chat"
```

Set the credential for the selected provider in the environment before running a workflow:

| Provider | Environment variable |
| --- | --- |
| DeepSeek | `DEEPSEEK_API_KEY` |
| OpenCode Zen | `OPENCODE_API_KEY` |
| OpenCode Go | `OPENCODE_API_KEY` |

Example provider setup:

```sh
export OPENCODE_API_KEY='replace-with-your-key'
koru model update opencode
koru model opencode/replace-with-a-model-id
koru model
```

Use a model identifier returned by the selected provider's catalog. To use OpenCode Go, use `opencode-go` in both the catalog and model-selection commands. Refreshing all catalogs requires credentials for all three services. A missing key or an invalid model selection is reported as an error; Koru does not fall back to a different provider or model.

Catalog data is cached below `$XDG_CACHE_HOME/koru/catalog/` (or `~/.cache/koru/catalog/` when unset). The cache is disposable; refresh it with `koru model update <SERVICE>` when a selected model is no longer present.

## Use the included workflows

### Ask a question

The `ask` example sends one tool-free request to the selected model and prints its response:

```sh
koru ask "Explain what src/main.rs does."
```

### Propose and run a shell action

The `shell` example takes one task argument, asks the selected model for a structured script proposal, validates the response, and presents the exact action for approval:

```sh
koru shell "List the five largest files in the current directory."
```

At the approval prompt, press `y` to authorize that exact action once. Any other answer denies it. Every proposed action needs its own approval. You can omit the task argument only in an interactive terminal; Koru then prompts for one bounded line of task input. The workflow cannot obtain approval in a redirected session.

After approval, Koru streams the captured standard output to standard output, standard error to standard error, and reports a nonzero exit status or signal as a diagnostic. It does not print the raw JSON result of the workflow. Koru runs the displayed script through its Linux process executor, bounds captured output, and removes inherited provider credentials from the child environment. The shell workflow cannot add arbitrary child environment variables.

### Review a staged-change plan

Stage only the files you want the planner to consider, inspect the staged summary, then run the workflow from the repository root:

```sh
git add -- src/module.rs
git diff --cached --stat
koru commit
```

Koru asks for approval before reading the staged diff. After approval, the selected model groups the staged change IDs and proposes Conventional Commit subjects. Rust validates complete change coverage and the messages, then Koru prints the plan and its identifiers for review. The planner reads staged changes only; unstaged edits are excluded.

The current `commit` workflow is plan-only. It does not run `git add`, `git commit`, `git reset`, or other Git write operations, and it does not modify the index, working tree, refs, or commit objects. You remain responsible for reviewing the plan and deciding whether to create commits. The planner supports at most 64 staged file changes and a 64 KiB staged diff. Git must be available on `PATH`.

## Write a workflow

A workflow is a Lua file that returns an API version 1 declaration. Save this example as `$XDG_CONFIG_HOME/koru/commands/hello.lua`:

```lua
return {
  api_version = 1,
  description = "Greet a person",
  arguments = {
    { name = "name", type = "string", required = true },
  },
  run = function(koru, args)
    return "Hello, " .. args.name
  end,
}
```

Validate and run it with:

```sh
koru check hello
koru hello Ada
```

Workflow arguments are positional and are validated before provider setup. A workflow that calls the model uses the host API, for example `koru.ai.ask(prompt)` or `koru.ai.ask_json({ prompt = ..., schema = ..., mode = "prompt_validate" })`. AI tool callbacks must be declared in the workflow and are checked against their JSON Schema contracts before dispatch.

The Lua environment is deliberately restricted. `io`, `os`, `package`, `debug`, the standard Lua loaders, and direct filesystem access are unavailable. `require` loads only modules included in the captured source bundle and declared with module directives. See [docs/declaration.md](docs/declaration.md) for declaration fields and [docs/lua-api.md](docs/lua-api.md) for the runtime API and supported JSON Schema subset.

## Troubleshooting

| Symptom | Resolution |
| --- | --- |
| No workflows are listed. | Install the workflow `.lua` files under `$XDG_CONFIG_HOME/koru/commands/`, or under `~/.config/koru/commands/` when `XDG_CONFIG_HOME` is unset. |
| A workflow reports that no model is selected. | Select a provider and model with `koru model <SERVICE>/<MODEL>`. |
| Koru reports a missing API key. | Set the credential for the selected service in the same environment that launches `koru`. |
| A model is not in the cached catalog. | Refresh that service with `koru model update <SERVICE>`, then select an ID printed by the command. |
| An approval request fails in a script or redirected terminal session. | Run the command directly in an interactive terminal. Approval cannot be piped or redirected. |
| `koru commit` reports no staged changes. | From the repository root, stage the intended paths with `git add -- PATH`, then run `git diff --cached --stat` and retry. |

Use `koru check [WORKFLOW]` to diagnose declaration problems without making provider requests or performing workflow effects.

## Security and current limits

- Provider credentials are read from environment variables. They are not stored in Koru configuration, exposed to Lua, or inherited by approved child processes.
- Workflows and model responses run under shared instruction, memory, time, request, and output limits. Effect requests pass through Rust-side validation and broker approval; the included shell workflow also validates the model proposal against a built-in schema before requesting execution.
- Shell actions require an interactive terminal and one explicit approval per action. Redirected sessions cannot approve actions.
- Shell execution and Git staged snapshots are Linux-only. The Git planner requires Git on `PATH`.
- The `commit` workflow returns a plan only. Automatic commit creation, grouped publication, and crash recovery are not implemented.
- Lua file effects, streaming responses, provider-native structured output, and durable stored permissions are not implemented.

See [docs/implementation.md](docs/implementation.md) for the implementation status and release gates, [docs/providers.md](docs/providers.md) for provider behavior, and [docs/DESIGN.md](docs/DESIGN.md) for the product requirements.

## Documentation

- [Design and release gates](docs/DESIGN.md)
- [Implementation status and measured evidence](docs/implementation.md)
- [Workflow declaration reference](docs/declaration.md)
- [Lua runtime API](docs/lua-api.md)
- [Provider configuration and behavior](docs/providers.md)
- [Implementation plans](docs/plans/)
