//! A single Rust-owned ledger shared by all nested command effects.
use crate::error::{ErrorCode, KoruError, Result};
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

static NEXT_RUN_ID: AtomicU64 = AtomicU64::new(1);

/// Hard-bounded whole-command resource allowances.
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    /// Logical model requests, including retry attempts.
    pub model_requests: u64,
    /// Total invocations of model-exposed tools.
    pub tool_calls: u64,
    /// Total prepared effect invocations.
    pub effects: u64,
    /// Total input/output bytes charged by adapters.
    pub bytes: u64,
    /// Cumulative Lua instruction budget charged by the VM hook.
    pub instructions: u64,
    /// Peak memory allowed in one Lua VM; a cap, not a cumulative debit.
    pub lua_memory_bytes: u64,
    /// Whole-command wall time including user approval.
    pub wall_time: Duration,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            model_requests: 32,
            tool_calls: 64,
            effects: 128,
            bytes: 8 * 1024 * 1024,
            instructions: 50_000_000,
            lua_memory_bytes: 64 * 1024 * 1024,
            wall_time: Duration::from_secs(600),
        }
    }
}
impl Limits {
    fn validate(self) -> Result<Self> {
        let hard = Self::default();
        if self.model_requests > hard.model_requests
            || self.tool_calls > hard.tool_calls
            || self.effects > hard.effects
            || self.bytes > hard.bytes
            || self.instructions > hard.instructions
            || self.lua_memory_bytes > hard.lua_memory_bytes
            || self.wall_time > hard.wall_time
        {
            return Err(KoruError::new(
                ErrorCode::Validation,
                "requested execution limits exceed hard ceilings",
            ));
        }
        Ok(self)
    }
}
/// One atomic reservation across all tracked resource dimensions.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Resources {
    /// Model request attempts.
    pub model_requests: u64,
    /// Model tool invocations.
    pub tool_calls: u64,
    /// Prepared effect invocations.
    pub effects: u64,
    /// Input/output bytes.
    pub bytes: u64,
    /// Lua instructions executed.
    pub instructions: u64,
}
impl Resources {
    /// A reservation with no additional resource use.
    pub const ZERO: Self = Self {
        model_requests: 0,
        tool_calls: 0,
        effects: 0,
        bytes: 0,
        instructions: 0,
    };
}
/// Terminal state cannot be reset by a workflow or its callbacks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunState {
    /// The command can reserve capacity and request effects.
    Active,
    /// A resource limit was reached.
    Exhausted,
    /// The cancellation signal was observed.
    Cancelled,
    /// The whole-command deadline elapsed.
    TimedOut,
}
#[derive(Debug)]
struct Ledger {
    usage: Resources,
    state: RunState,
}
#[derive(Debug)]
struct Shared {
    id: u64,
    command: String,
    source_digest: [u8; 32],
    limits: Limits,
    deadline: Instant,
    ledger: Mutex<Ledger>,
}
/// Immutable command identity and a synchronized, shared resource ledger.
#[derive(Debug, Clone)]
pub struct ExecutionContext(Arc<Shared>);
impl ExecutionContext {
    /// Start a run from a validated command/source snapshot and limited budget.
    pub fn new(
        command: impl Into<String>,
        source_digest: [u8; 32],
        limits: Limits,
        started: Instant,
    ) -> Result<Self> {
        let limits = limits.validate()?;
        let deadline = started
            .checked_add(limits.wall_time)
            .ok_or_else(|| KoruError::new(ErrorCode::Validation, "execution deadline overflows"))?;
        Ok(Self(Arc::new(Shared {
            id: NEXT_RUN_ID.fetch_add(1, Ordering::Relaxed),
            command: command.into(),
            source_digest,
            limits,
            deadline,
            ledger: Mutex::new(Ledger {
                usage: Resources::ZERO,
                state: RunState::Active,
            }),
        })))
    }
    /// Unique identifier within this CLI process.
    pub fn run_id(&self) -> u64 {
        self.0.id
    }
    /// Bound command name.
    pub fn command(&self) -> &str {
        &self.0.command
    }
    /// Bound captured source digest.
    pub fn source_digest(&self) -> [u8; 32] {
        self.0.source_digest
    }
    /// Validated immutable limits.
    pub fn limits(&self) -> Limits {
        self.0.limits
    }
    /// Whole-command deadline shared by approval, provider, and effects.
    pub fn deadline(&self) -> Instant {
        self.0.deadline
    }
    /// Reserve capacity before dispatch; any failure makes this run terminal.
    pub fn reserve(&self, request: Resources, now: Instant) -> Result<()> {
        let mut ledger = self.ledger()?;
        Self::check_state(&mut ledger, now, self.0.deadline)?;
        let usage = ledger.usage;
        let limits = self.0.limits;
        if over(
            usage.model_requests,
            request.model_requests,
            limits.model_requests,
        ) || over(usage.tool_calls, request.tool_calls, limits.tool_calls)
            || over(usage.effects, request.effects, limits.effects)
            || over(usage.bytes, request.bytes, limits.bytes)
            || over(
                usage.instructions,
                request.instructions,
                limits.instructions,
            )
        {
            ledger.state = RunState::Exhausted;
            return Err(KoruError::new(
                ErrorCode::BudgetExhausted,
                "execution budget exhausted",
            ));
        }
        ledger.usage = Resources {
            model_requests: usage.model_requests + request.model_requests,
            tool_calls: usage.tool_calls + request.tool_calls,
            effects: usage.effects + request.effects,
            bytes: usage.bytes + request.bytes,
            instructions: usage.instructions + request.instructions,
        };
        Ok(())
    }
    /// Mark this run cancelled; all clones observe the terminal state.
    pub fn cancel(&self) -> Result<()> {
        let mut ledger = self.ledger()?;
        if ledger.state == RunState::Active {
            ledger.state = RunState::Cancelled;
        }
        Ok(())
    }
    /// Check terminal status and the whole-command deadline without reserving.
    pub fn ensure_active(&self, now: Instant) -> Result<()> {
        let mut ledger = self.ledger()?;
        Self::check_state(&mut ledger, now, self.0.deadline)
    }
    /// Resource usage already reserved by this run.
    pub fn usage(&self) -> Result<Resources> {
        Ok(self.ledger()?.usage)
    }
    /// Current terminal or active state.
    pub fn state(&self) -> Result<RunState> {
        Ok(self.ledger()?.state)
    }
    fn ledger(&self) -> Result<std::sync::MutexGuard<'_, Ledger>> {
        self.0.ledger.lock().map_err(|_| {
            KoruError::new(ErrorCode::StateConflict, "execution ledger is unavailable")
        })
    }
    fn check_state(ledger: &mut Ledger, now: Instant, deadline: Instant) -> Result<()> {
        if ledger.state == RunState::Active && now >= deadline {
            ledger.state = RunState::TimedOut;
        }
        let (code, message) = match ledger.state {
            RunState::Active => return Ok(()),
            RunState::Exhausted => (ErrorCode::BudgetExhausted, "execution budget exhausted"),
            RunState::Cancelled => (ErrorCode::Cancelled, "execution cancelled"),
            RunState::TimedOut => (ErrorCode::Timeout, "execution deadline exceeded"),
        };
        Err(KoruError::new(code, message))
    }
}
fn over(used: u64, requested: u64, limit: u64) -> bool {
    used.checked_add(requested)
        .is_none_or(|total| total > limit)
}
