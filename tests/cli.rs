//! CLI contract tests run the actual binary with isolated user directories.
use std::{fs, process::Command};

#[test]
fn discovery_does_not_execute_scripts_and_inspection_labels_its_scope() {
    let root = tempfile::tempdir().unwrap();
    let commands = root.path().join("koru/commands");
    fs::create_dir_all(&commands).unwrap();
    fs::write(commands.join("hello.lua"), "error('must not run')").unwrap();
    let run = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_koru"))
            .env("XDG_CONFIG_HOME", root.path())
            .args(args)
            .output()
            .unwrap()
    };
    let listing = run(&[]);
    assert!(listing.status.success());
    assert!(String::from_utf8_lossy(&listing.stdout).contains("hello"));
    assert!(!listing.stdout.contains(&0x1b));
    let inspect = run(&["--inspect", "hello"]);
    assert!(inspect.status.success());
    assert!(String::from_utf8_lossy(&inspect.stdout).contains("not evaluated"));
    let check = run(&["check", "hello"]);
    assert!(!check.status.success());
    assert!(String::from_utf8_lossy(&check.stderr).contains("validation"));
    let workflow = run(&["hello"]);
    assert!(!workflow.status.success());
    assert!(String::from_utf8_lossy(&workflow.stderr).contains("validation"));
}

#[test]
fn check_validates_declarations_and_aggregates_failures() {
    let root = tempfile::tempdir().unwrap();
    let commands = root.path().join("koru/commands");
    fs::create_dir_all(&commands).unwrap();
    fs::write(
        commands.join("good.lua"),
        "return { api_version = 1, description = \"good\", run = function() end }",
    )
    .unwrap();
    fs::write(
        commands.join("bad.lua"),
        "return { api_version = 2, description = \"bad\", run = function() end }",
    )
    .unwrap();
    let run = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_koru"))
            .env("XDG_CONFIG_HOME", root.path())
            .args(args)
            .output()
            .unwrap()
    };
    let single = run(&["check", "good"]);
    assert!(single.status.success());
    assert!(String::from_utf8_lossy(&single.stdout).contains("good: ok"));
    assert!(!single.stdout.contains(&0x1b));

    let unsupported = run(&["check", "bad"]);
    assert!(!unsupported.status.success());
    assert!(String::from_utf8_lossy(&unsupported.stderr).contains("unsupported_capability"));

    let all = run(&["check"]);
    assert!(!all.status.success());
    let stdout = String::from_utf8_lossy(&all.stdout);
    assert!(stdout.contains("good: ok"));
    assert!(!stdout.contains("bad: ok"));
    let stderr = String::from_utf8_lossy(&all.stderr);
    assert!(stderr.contains("bad: unsupported_capability"));

    let too_many = run(&["check", "good", "bad"]);
    assert!(!too_many.status.success());
    assert!(String::from_utf8_lossy(&too_many.stderr).contains("at most one command"));
}
#[test]
fn missing_directory_is_empty_but_missing_selected_command_fails() {
    let root = tempfile::tempdir().unwrap();
    let run = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_koru"))
            .env("XDG_CONFIG_HOME", root.path())
            .args(args)
            .output()
            .unwrap()
    };
    assert!(run(&[]).status.success());
    assert!(!run(&["--inspect", "missing"]).status.success());
}

#[test]
fn redirected_help_has_no_ansi_sequences() {
    let output = Command::new(env!("CARGO_BIN_EXE_koru"))
        .arg("--help")
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(!output.stdout.contains(&0x1b));
    assert!(!output.stderr.contains(&0x1b));
}

#[test]
fn model_selection_persists_and_validates_variants() {
    let root = tempfile::tempdir().unwrap();
    let run = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_koru"))
            .env("XDG_CONFIG_HOME", root.path())
            .env("XDG_CACHE_HOME", root.path())
            .env_remove("DEEPSEEK_API_KEY")
            .env_remove("OPENCODE_API_KEY")
            .args(args)
            .output()
            .unwrap()
    };
    let initial = run(&["model"]);
    assert!(initial.status.success());
    let stdout = String::from_utf8_lossy(&initial.stdout);
    assert!(stdout.contains("provider: (none)"));
    assert!(stdout.contains("services: deepseek, opencode, opencode-go"));

    let selected = run(&["model", "deepseek/deepseek-chat"]);
    assert!(selected.status.success());
    let stdout = String::from_utf8_lossy(&selected.stdout);
    assert!(stdout.contains("provider: deepseek"));
    assert!(stdout.contains("model: deepseek-chat"));
    let config = fs::read_to_string(root.path().join("koru/config.toml")).unwrap();
    assert!(config.contains("schema_version = 1"));

    assert!(!run(&["variant", "high"]).status.success());
    let moved = run(&["model", "opencode-go/glm"]);
    assert!(moved.status.success());
    assert!(run(&["variant", "high"]).status.success());
    let cleared = run(&["model", "deepseek/again"]);
    assert!(cleared.status.success());
    assert!(String::from_utf8_lossy(&cleared.stdout).contains("variant: (none)"));
    assert!(String::from_utf8_lossy(&cleared.stderr).contains("cleared"));
}

#[test]
fn unknown_service_and_update_are_reported() {
    let root = tempfile::tempdir().unwrap();
    let run = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_koru"))
            .env("XDG_CONFIG_HOME", root.path())
            .env("XDG_CACHE_HOME", root.path())
            .env_remove("DEEPSEEK_API_KEY")
            .env_remove("OPENCODE_API_KEY")
            .args(args)
            .output()
            .unwrap()
    };
    assert!(!run(&["model", "nope/x"]).status.success());
    assert!(!run(&["variant"]).status.success());
    let update = run(&["model", "update", "deepseek"]);
    assert!(!update.status.success());
    assert!(String::from_utf8_lossy(&update.stderr).contains("DEEPSEEK_API_KEY"));
}

#[test]
fn selection_is_checked_against_the_cached_catalog() {
    let root = tempfile::tempdir().unwrap();
    let cache_dir = root.path().join("koru/catalog");
    fs::create_dir_all(&cache_dir).unwrap();
    fs::write(
        cache_dir.join("deepseek.json"),
        r#"{"schema_version":1,"fetched_at":1,"document":{"data":[{"id":"known"}]}}"#,
    )
    .unwrap();
    let run = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_koru"))
            .env("XDG_CONFIG_HOME", root.path())
            .env("XDG_CACHE_HOME", root.path())
            .args(args)
            .output()
            .unwrap()
    };
    let absent = run(&["model", "deepseek/absent"]);
    assert!(!absent.status.success());
    assert!(String::from_utf8_lossy(&absent.stderr).contains("not in the cached"));
    assert!(run(&["model", "deepseek/known"]).status.success());
}

#[test]
fn required_workflow_argument_fails_before_provider_setup() {
    let root = tempfile::tempdir().unwrap();
    let commands = root.path().join("koru/commands");
    fs::create_dir_all(&commands).unwrap();
    fs::write(
        commands.join("need_task.lua"),
        r#"return {
          api_version = 1,
          description = "Needs a task",
          arguments = {{ name = "task", type = "string", required = true }},
          run = function(koru, args) return args.task end,
        }"#,
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_koru"))
        .env("XDG_CONFIG_HOME", root.path())
        .env_remove("DEEPSEEK_API_KEY")
        .arg("need_task")
        .output()
        .unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("validation"), "{stderr}");
    assert!(stderr.contains("task"), "{stderr}");
}

#[test]
fn configured_lua_workflow_runs_and_prints_its_result() {
    let root = tempfile::tempdir().unwrap();
    let commands = root.path().join("koru/commands");
    fs::create_dir_all(&commands).unwrap();
    fs::write(
        commands.join("greet.lua"),
        r#"return {
          api_version = 1,
          description = "Greet someone",
          arguments = {{ name = "name", type = "string", required = true }},
          run = function(koru, args) return "Hello, " .. args.name end,
        }"#,
    )
    .unwrap();
    fs::write(
        root.path().join("koru/config.toml"),
        "schema_version = 1\nprovider = \"deepseek\"\nmodel = \"deepseek-chat\"\n",
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_koru"))
        .env("XDG_CONFIG_HOME", root.path())
        .env("XDG_CACHE_HOME", root.path())
        .env("DEEPSEEK_API_KEY", "fixture-key")
        .args(["greet", "Ada"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stdout, b"Hello, Ada\n");
}

#[test]
fn missing_model_selection_fails_before_workflow_execution() {
    let root = tempfile::tempdir().unwrap();
    let commands = root.path().join("koru/commands");
    fs::create_dir_all(&commands).unwrap();
    fs::write(
        commands.join("hello.lua"),
        "return { api_version = 1, description = 'hello', run = function() return 'ok' end }",
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_koru"))
        .env("XDG_CONFIG_HOME", root.path())
        .arg("hello")
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("select a model"));
}
