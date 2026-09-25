//! End-to-end declaration loading through the restricted Lua facade.
use koru::{
    error::{ErrorCode, KoruError},
    lua::LoadedCommand,
    runtime::{ExecutionContext, Limits},
    source::{SourceBundle, SourceLimits},
};
use std::{fs, time::Instant};
use tempfile::TempDir;

struct Fixture {
    _dir: TempDir,
    bundle: SourceBundle,
}
impl Fixture {
    fn new(entry: &str, modules: &[(&str, &str)]) -> Self {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("lib")).unwrap();
        fs::write(dir.path().join("demo.lua"), entry).unwrap();
        for (name, source) in modules {
            fs::write(dir.path().join("lib").join(format!("{name}.lua")), source).unwrap();
        }
        let bundle = SourceBundle::capture(dir.path(), "demo", SourceLimits::default()).unwrap();
        Self { _dir: dir, bundle }
    }
    fn context(&self, limits: Limits) -> ExecutionContext {
        ExecutionContext::new("demo", self.bundle.digest(), limits, Instant::now()).unwrap()
    }
    fn load(&self, limits: Limits) -> Result<LoadedCommand, KoruError> {
        let context = self.context(limits);
        LoadedCommand::load(&self.bundle, &context)
    }
    fn code(&self, limits: Limits) -> ErrorCode {
        match self.load(limits) {
            Ok(_) => panic!("expected loading to fail"),
            Err(error) => error.code(),
        }
    }
}
const HEADER: &str =
    "return { api_version = 1, description = \"demo\", run = function(koru, args) end }";

#[test]
fn restricted_environment_still_produces_a_valid_declaration() {
    let entry = format!(
        "{}\n{}",
        r#"
assert(os == nil and io == nil and package == nil and debug == nil, "restricted global exposed")
assert(load == nil and loadfile == nil and dofile == nil, "loader exposed")
assert(coroutine == nil, "coroutine exposed")
assert(print == nil and warn == nil and collectgarbage == nil, "base global exposed")
assert(type(require) == "function", "require missing")
"#,
        HEADER
    );
    let fixture = Fixture::new(&entry, &[]);
    assert!(fixture.load(Limits::default()).is_ok());
}

#[test]
fn run_is_validated_but_never_invoked() {
    let entry = r#"
return {
  api_version = 1,
  description = "demo",
  run = function() error("run must not be called during check") end,
}
"#;
    let fixture = Fixture::new(entry, &[]);
    assert!(fixture.load(Limits::default()).is_ok());
}

#[test]
fn missing_and_unknown_fields_are_rejected() {
    let fixture = Fixture::new(r#"return { api_version = 1, description = "demo" }"#, &[]);
    assert_eq!(fixture.code(Limits::default()), ErrorCode::Validation);

    let fixture = Fixture::new(
        r#"return { api_version = 1, description = "demo", run = function() end, bogus = 1 }"#,
        &[],
    );
    assert_eq!(fixture.code(Limits::default()), ErrorCode::Validation);
}

#[test]
fn unsupported_api_version_and_deferred_fields_fail_explicitly() {
    let fixture = Fixture::new(
        r#"return { api_version = 2, description = "demo", run = function() end }"#,
        &[],
    );
    assert_eq!(
        fixture.code(Limits::default()),
        ErrorCode::UnsupportedCapability
    );

    let fixture = Fixture::new(
        r#"return { api_version = 1, description = "demo", run = function() end, exemptions = {} }"#,
        &[],
    );
    assert_eq!(
        fixture.code(Limits::default()),
        ErrorCode::UnsupportedCapability
    );
}

#[test]
fn accepts_tool_declarations_and_requires_callbacks() {
    let valid = r#"
return {
  api_version = 1,
  description = "demo",
  tools = {
    { name = "lookup", description = "looks up", parameters = { type = "object" }, run = function(args) return args end },
  },
  run = function(koru, args) end,
}
"#;
    assert!(Fixture::new(valid, &[]).load(Limits::default()).is_ok());

    let missing_callback = r#"
return {
  api_version = 1,
  description = "demo",
  tools = { { name = "lookup", description = "looks up", parameters = {} } },
  run = function(koru, args) end,
}
"#;
    assert_eq!(
        Fixture::new(missing_callback, &[]).code(Limits::default()),
        ErrorCode::Validation
    );
}

#[test]
fn undeclared_require_is_rejected() {
    let entry = format!("local _ = require(\"missing\")\n{HEADER}");
    let fixture = Fixture::new(&entry, &[]);
    assert_eq!(fixture.code(Limits::default()), ErrorCode::Validation);
}

#[test]
fn declared_modules_read_captured_bytes_not_the_filesystem() {
    let entry = format!(
        "-- koru-module: util\nlocal util = require(\"util\")\nassert(util.ok == true, \"module not frozen\")\n{HEADER}"
    );
    let fixture = Fixture::new(&entry, &[("util", "return { ok = true }")]);
    fs::write(
        fixture._dir.path().join("lib/util.lua"),
        "return { ok = false }",
    )
    .unwrap();
    assert!(fixture.load(Limits::default()).is_ok());
}

#[test]
fn infinite_declarations_exhaust_the_instruction_budget() {
    let fixture = Fixture::new("while true do end", &[]);
    let limits = Limits {
        instructions: 50_000,
        ..Limits::default()
    };
    assert_eq!(fixture.code(limits), ErrorCode::BudgetExhausted);
}

#[test]
fn pcall_cannot_bypass_a_terminal_ledger() {
    let entry = format!("pcall(function() while true do end end)\n{HEADER}");
    let fixture = Fixture::new(&entry, &[]);
    let limits = Limits {
        instructions: 50_000,
        ..Limits::default()
    };
    assert_eq!(fixture.code(limits), ErrorCode::BudgetExhausted);
}

#[test]
fn memory_bombs_are_capped() {
    let entry = format!("local t = {{}} for i = 1, 100000000 do t[i] = i end\n{HEADER}");
    let fixture = Fixture::new(&entry, &[]);
    let limits = Limits {
        lua_memory_bytes: 512 * 1024,
        ..Limits::default()
    };
    assert_eq!(fixture.code(limits), ErrorCode::BudgetExhausted);
}

#[test]
fn cancelled_contexts_are_rejected() {
    let fixture = Fixture::new(HEADER, &[]);
    let context = fixture.context(Limits::default());
    context.cancel().unwrap();
    let error = match LoadedCommand::load(&fixture.bundle, &context) {
        Ok(_) => panic!("a cancelled context must not load"),
        Err(error) => error,
    };
    assert_eq!(error.code(), ErrorCode::Cancelled);
}
