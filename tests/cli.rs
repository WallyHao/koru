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
    assert!(String::from_utf8_lossy(&workflow.stderr).contains("unsupported_capability"));
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
