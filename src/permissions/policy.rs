//! In-memory policy and one-use broker authorization; persistence is a later gate.
use super::action::{Environment, Operation, PreparedAction};
use crate::{
    error::{ErrorCode, KoruError, Result},
    runtime::{ExecutionContext, Resources},
};
use std::{collections::BTreeSet, path::PathBuf, time::Instant};

/// Capabilities requested in script metadata; never a grant.
#[derive(Debug, Clone, Copy, Default)]
pub struct RequestedCapabilities {
    /// Script asks to invoke direct processes.
    pub direct_processes: bool,
}
/// A decision supplied only by the trusted terminal adapter.
#[derive(Debug, Clone, Copy)]
pub enum Decision {
    /// The user approved this one prepared action.
    ApproveOnce,
    /// The user denied or noninteractive mode cannot obtain confirmation.
    Deny,
}
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct ProcessGrant {
    policy_version: u64,
    command: String,
    source_digest: [u8; 32],
    executable: PathBuf,
    arguments: Vec<String>,
    cwd: PathBuf,
    environment: EnvironmentKey,
}
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum EnvironmentKey {
    Clean,
    Additions(Vec<(String, String)>),
}
impl From<&Environment> for EnvironmentKey {
    fn from(value: &Environment) -> Self {
        match value {
            Environment::Clean => Self::Clean,
            Environment::Additions(values) => Self::Additions(
                values
                    .iter()
                    .map(|(key, value)| (key.clone(), value.clone()))
                    .collect(),
            ),
        }
    }
}
/// Host-owned policy snapshot; durable storage is not yet implemented.
#[derive(Debug)]
pub struct Policy {
    version: u64,
    process_grants: BTreeSet<ProcessGrant>,
}
impl Policy {
    /// Start an empty policy at a positive schema/policy version.
    pub fn new(version: u64) -> Result<Self> {
        if version == 0 {
            return Err(KoruError::new(
                ErrorCode::Validation,
                "policy version must be positive",
            ));
        }
        Ok(Self {
            version,
            process_grants: BTreeSet::new(),
        })
    }
    /// Record a reviewed exact direct-process exemption in this memory snapshot.
    pub fn grant_exact_process(
        &mut self,
        action: &PreparedAction,
        context: &ExecutionContext,
    ) -> Result<()> {
        if !action.matches(context) {
            return Err(KoruError::new(
                ErrorCode::StateConflict,
                "action context changed",
            ));
        }
        let grant = self.process_key(action).ok_or_else(|| {
            KoruError::new(
                ErrorCode::Validation,
                "only direct processes can receive stored exemptions",
            )
        })?;
        self.process_grants.insert(grant);
        Ok(())
    }
    fn has_grant(&self, action: &PreparedAction) -> bool {
        self.process_key(action)
            .is_some_and(|grant| self.process_grants.contains(&grant))
    }
    fn process_key(&self, action: &PreparedAction) -> Option<ProcessGrant> {
        let Operation::Process {
            executable,
            arguments,
            cwd,
            environment,
        } = &action.operation
        else {
            return None;
        };
        Some(ProcessGrant {
            policy_version: self.version,
            command: action.command.clone(),
            source_digest: action.source_digest,
            executable: executable.clone(),
            arguments: arguments.clone(),
            cwd: cwd.clone(),
            environment: EnvironmentKey::from(environment),
        })
    }
}
/// The broker authorizes a prepared action against host policy or a terminal decision.
pub struct Broker;
impl Broker {
    /// Return a one-use authorization for the unchanged action and context.
    pub fn authorize(
        action: PreparedAction,
        context: &ExecutionContext,
        policy: &Policy,
        decision: Decision,
        now: Instant,
    ) -> Result<ApprovedAction> {
        context.ensure_active(now)?;
        if !action.matches(context) {
            return Err(KoruError::new(
                ErrorCode::StateConflict,
                "action context changed",
            ));
        }
        if !policy.has_grant(&action) && !matches!(decision, Decision::ApproveOnce) {
            return Err(KoruError::new(
                ErrorCode::PermissionDenied,
                "confirmation required for this action",
            ));
        }
        Ok(ApprovedAction {
            action,
            begun: false,
        })
    }
}
/// A broker-issued, single-use authorization. It does not itself execute an effect.
#[derive(Debug)]
pub struct ApprovedAction {
    action: PreparedAction,
    begun: bool,
}
impl ApprovedAction {
    /// The exact action approved; adapters must execute this object.
    pub fn action(&self) -> &PreparedAction {
        &self.action
    }
    /// Recheck context and reserve one effect immediately before dispatch.
    pub fn begin(&mut self, context: &ExecutionContext, now: Instant) -> Result<&PreparedAction> {
        if self.begun || !self.action.matches(context) {
            return Err(KoruError::new(
                ErrorCode::StateConflict,
                "authorization already used or bound to another context",
            ));
        }
        context.reserve(
            Resources {
                effects: 1,
                ..Resources::ZERO
            },
            now,
        )?;
        self.begun = true;
        Ok(&self.action)
    }
}
