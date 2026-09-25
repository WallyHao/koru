//! Load one captured command declaration under the sandboxed VM.
use super::{bridge, declaration, sandbox::Sandbox};
use crate::{
    ai::{AiService, ToolSpec},
    declaration::CommandDeclaration,
    error::{ErrorCode, KoruError, Result},
    json::JsonValue,
    runtime::ExecutionContext,
    source::SourceBundle,
};
use mlua::{Function, Value};
use std::collections::BTreeMap;

/// A declared tool callback retained for VM-owner dispatch.
pub(super) struct ToolEntry {
    pub(super) spec: ToolSpec,
    pub(super) callback: Function,
}

/// A validated declaration still bound to the restricted VM that produced it.
///
/// The VM, `run` function, and tool callbacks are retained so the workflow can be
/// executed without re-evaluating the source bundle.
pub struct LoadedCommand {
    pub(super) declaration: CommandDeclaration,
    pub(super) run: Function,
    pub(super) tools: BTreeMap<String, ToolEntry>,
    pub(super) sandbox: Sandbox,
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
        let (declaration, run, callbacks) = declaration::convert(&table)?;
        let mut tools = BTreeMap::new();
        for (tool, callback) in declaration.tools().iter().zip(callbacks) {
            tools.insert(
                tool.name.clone(),
                ToolEntry {
                    spec: ToolSpec {
                        name: tool.name.clone(),
                        description: tool.description.clone(),
                        parameters: tool.parameters.clone(),
                    },
                    callback,
                },
            );
        }
        Ok(Self {
            declaration,
            run,
            tools,
            sandbox,
        })
    }
    /// The validated declaration.
    pub fn declaration(&self) -> &CommandDeclaration {
        &self.declaration
    }
    /// Execute the workflow against a service and return its result as JSON.
    pub fn run(self, service: Box<dyn AiService>, args: &JsonValue) -> Result<JsonValue> {
        bridge::run(self, service, args)
    }
}
