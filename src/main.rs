//! CLI composition root for currently implemented source discovery.
use clap::Parser;
use koru::{
    error::{ErrorCode, KoruError, Result},
    paths::UserPaths,
    source::{SourceBundle, SourceLimits, discover},
};

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
    /// A command to run; workflow execution is not implemented yet.
    command: Option<String>,
    /// Command arguments, reserved for the workflow runtime.
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
    if let Some(name) = cli.command {
        return Err(KoruError::new(
            ErrorCode::UnsupportedCapability,
            format!("{name}: workflow execution and check are not implemented yet"),
        ));
    }
    for name in discover(&commands)? {
        println!("{name}");
    }
    Ok(())
}
fn main() {
    if let Err(error) = run(Cli::parse()) {
        eprintln!("koru [{}]: {}", error.code().as_str(), error.message());
        std::process::exit(1);
    }
}
