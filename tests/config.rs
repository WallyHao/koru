//! Configuration contract: strict flat parsing, atomic writes, scoped lock.
use koru::{config::Config, error::ErrorCode, paths::UserPaths};
use std::{fs, thread};

fn parse_code(text: &str) -> ErrorCode {
    Config::parse(text).unwrap_err().code()
}

#[test]
fn parses_the_flat_subset() {
    let config = Config::parse(
        "# Koru configuration\n\nschema_version = 1\nprovider = \"deepseek\"\nmodel = \"deepseek-chat\"\nvariant = \"high\"\n",
    )
    .unwrap();
    assert_eq!(config.provider.as_deref(), Some("deepseek"));
    assert_eq!(config.model.as_deref(), Some("deepseek-chat"));
    assert_eq!(config.variant.as_deref(), Some("high"));
}

#[test]
fn minimal_configuration_is_allowed() {
    let config = Config::parse("schema_version = 1\n").unwrap();
    assert_eq!(config, Config::default());
}

#[test]
fn provider_without_a_model_is_allowed_but_variant_needs_one() {
    assert!(Config::parse("schema_version = 1\nprovider = \"deepseek\"\n").is_ok());
    assert_eq!(
        parse_code("schema_version = 1\nprovider = \"deepseek\"\nvariant = \"high\"\n"),
        ErrorCode::Validation
    );
}

#[test]
fn rejects_malformed_lines_and_unknown_keys() {
    for text in [
        "provider = \"x\"\n",
        "schema_version = 1\nbogus = \"x\"\n",
        "schema_version = 1\nprovider = \"x\"\nprovider = \"y\"\n",
        "schema_version 1\n",
        "schema_version = one\n",
        "schema_version = 1\nprovider = deepseek\n",
        "schema_version = 1\nprovider = \"x\n",
        "schema_version = 1\nprovider = \"x\" trailing\n",
    ] {
        assert_eq!(parse_code(text), ErrorCode::Validation, "text: {text:?}");
    }
}

#[test]
fn rejects_unsupported_schema_versions() {
    assert_eq!(parse_code("schema_version = 2\n"), ErrorCode::Validation);
    assert_eq!(parse_code("schema_version = 0\n"), ErrorCode::Validation);
}

#[test]
fn rejects_unsupported_escapes_and_control_characters() {
    assert_eq!(
        parse_code("schema_version = 1\nprovider = \"a\\qb\"\n"),
        ErrorCode::Validation
    );
    assert_eq!(
        parse_code("schema_version = 1\nprovider = \"\"\n"),
        ErrorCode::Validation
    );
}

#[test]
fn round_trips_through_render() {
    let config = Config {
        provider: Some("opencode-go".to_owned()),
        model: Some("model/with\"quote".to_owned()),
        variant: None,
    };
    let rendered = config.render().unwrap();
    assert_eq!(Config::parse(&rendered).unwrap(), config);
}

#[test]
fn missing_file_loads_defaults() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("koru/config.toml");
    assert_eq!(Config::load(&path).unwrap(), Config::default());
}

#[test]
fn update_persists_and_leaves_no_artifacts() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("koru/config.toml");
    let updated = Config::update(&path, |config| {
        config.provider = Some("deepseek".to_owned());
        config.model = Some("deepseek-chat".to_owned());
    })
    .unwrap();
    assert_eq!(updated.provider.as_deref(), Some("deepseek"));
    assert_eq!(Config::load(&path).unwrap(), updated);
    let entries = fs::read_dir(path.parent().unwrap())
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect::<Vec<_>>();
    assert_eq!(
        entries.len(),
        1,
        "only config.toml should remain: {entries:?}"
    );
}

#[test]
fn concurrent_updates_do_not_lose_a_field() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("koru/config.toml");
    thread::scope(|scope| {
        let provider_path = path.clone();
        scope.spawn(move || {
            for _ in 0..25 {
                Config::update(&provider_path, |config| {
                    config.provider = Some("deepseek".to_owned());
                })
                .unwrap();
            }
        });
        let model_path = path.clone();
        scope.spawn(move || {
            for _ in 0..25 {
                Config::update(&model_path, |config| {
                    config.model = Some("deepseek-chat".to_owned());
                })
                .unwrap();
            }
        });
    });
    let config = Config::load(&path).unwrap();
    assert_eq!(config.provider.as_deref(), Some("deepseek"));
    assert_eq!(config.model.as_deref(), Some("deepseek-chat"));
}

#[test]
fn config_path_is_under_the_config_directory() {
    let paths = UserPaths {
        config: "/tmp/xdg/koru".into(),
        cache: "/tmp/cache".into(),
        state: "/tmp/state".into(),
    };
    assert_eq!(
        Config::path(&paths),
        std::path::PathBuf::from("/tmp/xdg/koru/config.toml")
    );
}
