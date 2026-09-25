//! Construct the restricted Lua VM and install its execution controls.
//!
//! The base library is always opened by mlua, so every unsafe base global is
//! removed explicitly before any command source is evaluated. Limits are charged
//! through the shared [`ExecutionContext`] ledger; terminal states are decided by
//! the ledger, not by a Lua error a script might catch.
use super::{error, loader};
use crate::{
    error::Result,
    runtime::{ExecutionContext, Resources},
    source::SourceBundle,
};
use mlua::{HookTriggers, Lua, LuaOptions, StdLib, Value, VmState};
use std::{convert::TryFrom, time::Instant};

/// Instruction count charged to the ledger on every hook tick.
const INSTRUCTION_QUANTUM: u32 = 10_000;
const FORBIDDEN_GLOBALS: &[&str] = &[
    "collectgarbage",
    "coroutine",
    "debug",
    "dofile",
    "io",
    "load",
    "loadfile",
    "module",
    "os",
    "package",
    "print",
    "warn",
];

/// A Lua VM whose libraries, limits, and loader are Koru-controlled.
pub(super) struct Sandbox {
    lua: Lua,
    context: ExecutionContext,
}
impl Sandbox {
    /// Build a VM for one captured command; no command source runs yet.
    pub(super) fn new(bundle: &SourceBundle, context: &ExecutionContext) -> Result<Self> {
        context.ensure_active(Instant::now())?;
        let libs = StdLib::TABLE | StdLib::STRING | StdLib::UTF8 | StdLib::MATH;
        let lua = Lua::new_with(libs, LuaOptions::default())
            .map_err(|error| error::map(context, error, "cannot create the Lua VM"))?;
        let globals = lua.globals();
        for name in FORBIDDEN_GLOBALS {
            globals
                .set(*name, Value::Nil)
                .map_err(|error| error::map(context, error, "cannot restrict Lua globals"))?;
        }
        loader::install_require(&lua, bundle, context)?;
        let memory = usize::try_from(context.limits().lua_memory_bytes)
            .map_err(|_| error::invalid("Lua memory limit is too large for this platform"))?;
        lua.set_memory_limit(memory)
            .map_err(|error| error::map(context, error, "cannot set the Lua memory limit"))?;
        install_hook(&lua, context)?;
        Ok(Self {
            lua,
            context: context.clone(),
        })
    }
    /// Evaluate the captured entry chunk and return its declaration value.
    pub(super) fn evaluate_entry(&self, bundle: &SourceBundle) -> Result<Value> {
        let value = self
            .lua
            .load(bundle.entry().bytes())
            .set_name(format!("@{}.lua", bundle.command()))
            .eval::<Value>()
            .map_err(|error| error::map(&self.context, error, "declaration evaluation failed"))?;
        // A script can catch the hook abort with pcall; the ledger still decides.
        self.context.ensure_active(Instant::now())?;
        Ok(value)
    }
    /// The execution context bound to this VM.
    pub(super) fn context(&self) -> &ExecutionContext {
        &self.context
    }
    /// Consume the sandbox, keeping the VM alive for workflow execution.
    pub(super) fn into_lua(self) -> Lua {
        self.lua
    }
    #[cfg(test)]
    pub(super) fn lua(&self) -> &Lua {
        &self.lua
    }
}

fn install_hook(lua: &Lua, context: &ExecutionContext) -> Result<()> {
    let hook_context = context.clone();
    lua.set_global_hook(
        HookTriggers::new().every_nth_instruction(INSTRUCTION_QUANTUM),
        move |_lua, _debug| {
            let now = Instant::now();
            hook_context
                .reserve(
                    Resources {
                        instructions: u64::from(INSTRUCTION_QUANTUM),
                        ..Resources::ZERO
                    },
                    now,
                )
                .map_err(|error| mlua::Error::RuntimeError(error.message().to_owned()))?;
            hook_context
                .ensure_active(now)
                .map_err(|error| mlua::Error::RuntimeError(error.message().to_owned()))?;
            Ok(VmState::Continue)
        },
    )
    .map_err(|error| error::map(context, error, "cannot install the Lua instruction hook"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        runtime::{Limits, RunState},
        source::SourceLimits,
    };
    use std::{fs, time::Duration};
    use tempfile::TempDir;

    fn bundle(script: &[u8]) -> (TempDir, SourceBundle) {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("lib")).unwrap();
        fs::write(dir.path().join("demo.lua"), script).unwrap();
        let bundle = SourceBundle::capture(dir.path(), "demo", SourceLimits::default()).unwrap();
        (dir, bundle)
    }
    fn sandbox(script: &[u8], limits: Limits) -> (TempDir, ExecutionContext, Sandbox) {
        let (dir, bundle) = bundle(script);
        let context =
            ExecutionContext::new("demo", bundle.digest(), limits, Instant::now()).unwrap();
        let sandbox = Sandbox::new(&bundle, &context).unwrap();
        (dir, context, sandbox)
    }
    fn global_is_nil(sandbox: &Sandbox, name: &str) -> bool {
        sandbox.lua().globals().get::<Value>(name).unwrap().is_nil()
    }

    #[test]
    fn base_library_is_reduced_to_an_explicit_allowlist() {
        let (_dir, _context, sandbox) = sandbox(b"return {}", Limits::default());
        for name in FORBIDDEN_GLOBALS {
            assert!(global_is_nil(&sandbox, name), "{name} must be removed");
        }
        for name in ["table", "string", "math", "utf8", "pairs", "pcall", "type"] {
            assert!(!global_is_nil(&sandbox, name), "{name} must remain");
        }
        assert!(!global_is_nil(&sandbox, "require"));
    }

    #[test]
    fn instruction_budget_aborts_an_infinite_loop() {
        let limits = Limits {
            instructions: 50_000,
            ..Limits::default()
        };
        let (_dir, context, sandbox) = sandbox(b"return {}", limits);
        let error = sandbox.lua().load("while true do end").exec().unwrap_err();
        assert!(matches!(error, mlua::Error::RuntimeError(_)));
        assert_eq!(context.state().unwrap(), RunState::Exhausted);
    }

    #[test]
    fn deadline_is_charged_while_lua_runs() {
        let limits = Limits {
            wall_time: Duration::from_millis(5),
            ..Limits::default()
        };
        let (_dir, context, sandbox) = sandbox(b"return {}", limits);
        sandbox.lua().load("while true do end").exec().unwrap_err();
        assert_eq!(context.state().unwrap(), RunState::TimedOut);
    }

    #[test]
    fn memory_limit_rejects_large_allocations() {
        let limits = Limits {
            lua_memory_bytes: 512 * 1024,
            ..Limits::default()
        };
        let (_dir, _context, sandbox) = sandbox(b"return {}", limits);
        let error = sandbox
            .lua()
            .load("local t = {} for i = 1, 100000000 do t[i] = i end")
            .exec()
            .unwrap_err();
        assert!(matches!(error, mlua::Error::MemoryError(_)));
    }

    #[test]
    fn pcall_cannot_resume_after_the_ledger_is_terminal() {
        let limits = Limits {
            instructions: 50_000,
            ..Limits::default()
        };
        let (_dir, context, sandbox) = sandbox(b"return {}", limits);
        sandbox
            .lua()
            .load("pcall(function() while true do end end)")
            .exec()
            .unwrap();
        assert_eq!(context.state().unwrap(), RunState::Exhausted);
        let error = sandbox
            .lua()
            .load("for i = 1, 100000 do end")
            .exec()
            .unwrap_err();
        assert!(matches!(error, mlua::Error::RuntimeError(_)));
    }

    #[test]
    fn global_hook_covers_mlua_created_threads() {
        let limits = Limits {
            instructions: 20_000,
            ..Limits::default()
        };
        let (_dir, context, sandbox) = sandbox(b"return {}", limits);
        let function = sandbox
            .lua()
            .load("for i = 1, 20000000 do end return 1")
            .into_function()
            .unwrap();
        let thread = sandbox.lua().create_thread(function).unwrap();
        let result = thread.resume::<Value>(());
        assert!(result.is_err());
        assert_eq!(context.state().unwrap(), RunState::Exhausted);
    }

    #[test]
    fn cancelled_context_is_rejected_before_any_load() {
        let (dir, bundle) = bundle(b"return {}");
        let context =
            ExecutionContext::new("demo", bundle.digest(), Limits::default(), Instant::now())
                .unwrap();
        context.cancel().unwrap();
        let error = match Sandbox::new(&bundle, &context) {
            Ok(_) => panic!("a cancelled context must not build a sandbox"),
            Err(error) => error,
        };
        assert_eq!(error.code(), crate::error::ErrorCode::Cancelled);
        drop(dir);
    }
}
