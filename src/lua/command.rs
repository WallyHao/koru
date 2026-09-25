//! Load one captured command declaration under the sandboxed VM.
use super::{bridge, declaration, sandbox::Sandbox};
use crate::{
    ai::{AiService, ToolSpec},
    declaration::CommandDeclaration,
    error::{ErrorCode, KoruError, Result},
    json::JsonValue,
    permissions::{Decision, PreparedAction},
    runtime::ExecutionContext,
    schema::JsonSchema,
    source::SourceBundle,
};
use mlua::{Function, Value};
use std::{collections::BTreeMap, time::Instant};

/// Host-owned source of approval decisions for prepared effects.
pub trait ApprovalProvider {
    /// Decide whether one exact prepared action may run before `deadline`.
    fn decide(&mut self, action: &PreparedAction, deadline: Instant) -> Result<Decision>;
}

struct TerminalApproval;
impl ApprovalProvider for TerminalApproval {
    fn decide(&mut self, action: &PreparedAction, deadline: Instant) -> Result<Decision> {
        crate::terminal::approve(action, deadline)
    }
}

/// A declared tool callback retained for VM-owner dispatch.
pub(super) struct ToolEntry {
    pub(super) spec: ToolSpec,
    pub(super) result: Option<JsonSchema>,
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
                tool.name().to_owned(),
                ToolEntry {
                    spec: ToolSpec {
                        name: tool.name().to_owned(),
                        description: tool.description().to_owned(),
                        parameters: tool.parameters().clone(),
                    },
                    result: tool.result().cloned(),
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
        self.run_with_approval(service, args, &mut TerminalApproval)
    }

    /// Execute with an explicit host approval adapter.
    pub fn run_with_approval(
        self,
        service: Box<dyn AiService>,
        args: &JsonValue,
        approval: &mut dyn ApprovalProvider,
    ) -> Result<JsonValue> {
        bridge::run(self, service, args, approval)
    }
}
