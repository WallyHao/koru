//! CLI composition root for source discovery and bounded declaration checking.
use clap::Parser;
use koru::{
    error::{ErrorCode, KoruError, Result},
    lua::LoadedCommand,
    paths::UserPaths,
    runtime::{ExecutionContext, Limits},
    source::{SourceBundle, SourceLimits, discover},
};
use std::{path::Path, time::Instant};

const BUILTINS: &[&str] = &[
    "check", "model", "variant", "recover", "help", "list", "inspect",
];

#[derive(Debug, Parser)]
#[command(
    name = "koru",
    version,
    about = "Permission-aware Lua workflows (foundation preview)"
)]
struct Cli {
    /// Inspect a captured source bundle without evaluating or validating Lua.
    #[arg(long, value_name = "COMMAND")]
    inspect: Option<String>,
    /// A command or builtin to run.
    command: Option<String>,
    /// Command arguments, reserved for the workflow runtime and `check`.
    #[arg(trailing_var_arg = true)]
    args: Vec<String>,
}
fn run(cli: Cli) -> Result<()> {
    let paths = UserPaths::from_environment()?;
    let commands = paths.commands();
    if let Some(name) = cli.inspect {
        if cli.command.is_some() || !cli.args.is_empty() {
            return Err(KoruError::new(
                ErrorCode::Validation,
                "--inspect cannot be combined with a workflow command",
            ));
        }
        let bundle = SourceBundle::capture(&commands, &name, SourceLimits::default())?;
        println!("command: {}", bundle.command());
        println!("source_sha256: {}", bundle.digest_hex());
        println!("modules: {}", bundle.modules().len());
        println!("scope: captured only; Lua was not evaluated");
        return Ok(());
    }
    match cli.command.as_deref() {
        None => {
            for name in discover(&commands)? {
                println!("{name}");
            }
            Ok(())
        }
        Some("check") => run_check(&commands, &cli.args),
        Some(name) if BUILTINS.contains(&name) => Err(KoruError::new(
            ErrorCode::UnsupportedCapability,
            format!("{name}: this builtin is not implemented yet"),
        )),
        Some(name) => Err(KoruError::new(
            ErrorCode::UnsupportedCapability,
            format!("{name}: workflow execution is not implemented yet"),
        )),
    }
}
fn run_check(commands: &Path, args: &[String]) -> Result<()> {
    match args {
        [] => {
            let mut failed = 0usize;
            for name in discover(commands)? {
                match check_one(commands, &name) {
                    Ok(()) => println!("{name}: ok"),
                    Err(error) => {
                        failed += 1;
                        eprintln!("{name}: {}: {}", error.code().as_str(), error.message());
                    }
                }
            }
            if failed > 0 {
                return Err(KoruError::new(
                    ErrorCode::Validation,
                    format!("{failed} command(s) failed validation"),
                ));
            }
            Ok(())
        }
        [name] => {
            check_one(commands, name)?;
            println!("{name}: ok");
            Ok(())
        }
        _ => Err(KoruError::new(
            ErrorCode::Validation,
            "check accepts at most one command name",
        )),
    }
}
fn check_one(commands: &Path, name: &str) -> Result<()> {
    let bundle = SourceBundle::capture(commands, name, SourceLimits::default())?;
    let context = ExecutionContext::new(name, bundle.digest(), Limits::default(), Instant::now())?;
    let command = LoadedCommand::load(&bundle, &context)?;
    debug_assert_eq!(command.declaration().api_version(), koru::API_VERSION);
    Ok(())
}
fn main() {
    if let Err(error) = run(Cli::parse()) {
        eprintln!("koru [{}]: {}", error.code().as_str(), error.message());
        std::process::exit(1);
    }
}
