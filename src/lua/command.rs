//! Load one captured command declaration under the sandboxed VM.
use super::{declaration, sandbox::Sandbox};
use crate::{
    declaration::CommandDeclaration,
    error::{ErrorCode, KoruError, Result},
    runtime::ExecutionContext,
    source::SourceBundle,
};
use mlua::{Function, Value};

/// A validated declaration still bound to the restricted VM that produced it.
///
/// The VM and `run` function are retained so a future workflow runtime can invoke
/// `run` without re-evaluating the source bundle.
pub struct LoadedCommand {
    declaration: CommandDeclaration,
    #[allow(dead_code)]
    run: Function,
    #[allow(dead_code)]
    sandbox: Sandbox,
}
impl LoadedCommand {
    /// Evaluate and validate one captured command; `run` is never invoked.
    pub fn load(bundle: &SourceBundle, context: &ExecutionContext) -> Result<Self> {
        let sandbox = Sandbox::new(bundle, context)?;
        let value = sandbox.evaluate_entry(bundle)?;
        let table = match value {
            Value::Table(table) => table,
            _ => {
                return Err(KoruError::new(
                    ErrorCode::Validation,
                    "a command declaration must return a table",
                ));
            }
        };
        let (declaration, run) = declaration::convert(&table)?;
        Ok(Self {
            declaration,
            run,
            sandbox,
        })
    }
    /// The validated declaration.
    pub fn declaration(&self) -> &CommandDeclaration {
        &self.declaration
    }
}
