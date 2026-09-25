//! CLI composition root for source discovery, declaration checks, and selection.
use clap::Parser;
use koru::{
    config::Config,
    error::{ErrorCode, KoruError, Result},
    lua::LoadedCommand,
    paths::UserPaths,
    provider::catalog::{Catalog, ServiceId, UnavailableCatalogSource, declared_variants},
    runtime::{ExecutionContext, Limits},
    source::{SourceBundle, SourceLimits, discover},
};
use std::{
    path::Path,
    time::{Instant, SystemTime},
};

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
        Some("model") => run_model(&paths, &cli.args),
        Some("variant") => run_variant(&paths, &cli.args),
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

fn print_selection(config: &Config) {
    println!(
        "provider: {}",
        config.provider.as_deref().unwrap_or("(none)")
    );
    println!("model: {}", config.model.as_deref().unwrap_or("(none)"));
    println!("variant: {}", config.variant.as_deref().unwrap_or("(none)"));
}

fn run_model(paths: &UserPaths, args: &[String]) -> Result<()> {
    let path = Config::path(paths);
    let args = args.iter().map(String::as_str).collect::<Vec<_>>();
    match args.as_slice() {
        [] => {
            print_selection(&Config::load(&path)?);
            let services = ServiceId::ALL
                .iter()
                .map(|service| service.name())
                .collect::<Vec<_>>()
                .join(", ");
            println!("services: {services}");
            Ok(())
        }
        [first] if *first == "update" => Err(update_usage()),
        ["update", "--all"] => run_model_update(&ServiceId::ALL, &path),
        ["update", service] => {
            let service = parse_service(service)?;
            run_model_update(&[service], &path)
        }
        [selector] => select_model(&path, selector),
        _ => Err(KoruError::new(
            ErrorCode::Validation,
            "model accepts at most one selector",
        )),
    }
}

fn select_model(path: &Path, selector: &str) -> Result<()> {
    let (service, model) = match selector.split_once('/') {
        Some((service, model)) => {
            if model.is_empty() {
                return Err(KoruError::new(
                    ErrorCode::Validation,
                    "a model selector of the form <service>/<model> needs a model name",
                ));
            }
            (parse_service(service)?, Some(model.to_owned()))
        }
        None => (parse_service(selector)?, None),
    };
    let mut notice = None;
    let updated = Config::update(path, |config| {
        config.provider = Some(service.name().to_owned());
        if let Some(model) = model.clone() {
            config.model = Some(model);
        } else {
            config.model = None;
        }
        if let Some(variant) = config.variant.as_deref()
            && !declared_variants(service).contains(&variant)
        {
            notice = Some(format!(
                "variant {variant:?} is not valid for {}; cleared",
                service.name()
            ));
            config.variant = None;
        }
    })?;
    if let Some(notice) = notice {
        eprintln!("{notice}");
    }
    print_selection(&updated);
    Ok(())
}

fn run_model_update(services: &[ServiceId], path: &Path) -> Result<()> {
    let mut catalog = Catalog::default();
    let fetched_at = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|value| value.as_secs())
        .unwrap_or(0);
    for service in services {
        catalog.refresh(*service, &UnavailableCatalogSource, fetched_at)?;
        for entry in catalog.entries(*service) {
            println!("{}: {}", service.name(), entry.id);
        }
    }
    let selected = Config::load(path)?;
    if let (Some(provider), Some(model)) = (selected.provider.as_deref(), selected.model.as_deref())
        && let Some(service) = ServiceId::parse(provider)
        && catalog.model(service, model).is_none()
        && services.contains(&service)
    {
        return Err(KoruError::new(
            ErrorCode::Validation,
            format!("selected model {provider}/{model} is not in the fetched catalog"),
        ));
    }
    Ok(())
}

fn run_variant(paths: &UserPaths, args: &[String]) -> Result<()> {
    let path = Config::path(paths);
    let config = Config::load(&path)?;
    let (Some(provider), Some(model)) = (config.provider.as_deref(), config.model.as_deref())
    else {
        return Err(KoruError::new(
            ErrorCode::Validation,
            "select a model before choosing a variant",
        ));
    };
    let service = parse_service(provider)?;
    let valid = declared_variants(service);
    let args = args.iter().map(String::as_str).collect::<Vec<_>>();
    match args.as_slice() {
        [] => {
            print_selection(&config);
            println!(
                "valid variants: {}",
                if valid.is_empty() {
                    "(none)".to_owned()
                } else {
                    valid.join(", ")
                }
            );
            Ok(())
        }
        [name] => {
            let name = *name;
            if !valid.contains(&name) {
                return Err(KoruError::new(
                    ErrorCode::Validation,
                    format!(
                        "variant {name:?} is not valid for {provider}/{model}; valid: {}",
                        if valid.is_empty() {
                            "(none)".to_owned()
                        } else {
                            valid.join(", ")
                        }
                    ),
                ));
            }
            let updated = Config::update(&path, |config| {
                config.variant = Some(name.to_owned());
            })?;
            print_selection(&updated);
            Ok(())
        }
        _ => Err(KoruError::new(
            ErrorCode::Validation,
            "variant accepts at most one name",
        )),
    }
}

fn parse_service(name: &str) -> Result<ServiceId> {
    ServiceId::parse(name).ok_or_else(|| {
        let known = ServiceId::ALL
            .iter()
            .map(|service| service.name())
            .collect::<Vec<_>>()
            .join(", ");
        KoruError::new(
            ErrorCode::Validation,
            format!("unknown service {name:?}; implemented services: {known}"),
        )
    })
}

fn update_usage() -> KoruError {
    KoruError::new(
        ErrorCode::Validation,
        "usage: koru model update <service>|--all",
    )
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
