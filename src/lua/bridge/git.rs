//! Brokered read-only Git snapshot requests and Rust plan validation.
use super::{Pending, PendingRequest};
use crate::{
    error::Result,
    json::{JsonLimits, JsonValue},
    lua::{ApprovalProvider, error},
    runtime::{ExecutionContext, Resources},
};
use mlua::{Lua, Table, Value};
use sha2::{Digest, Sha256};
use std::{
    cell::{Cell, RefCell},
    collections::BTreeSet,
    rc::Rc,
};

/// IDs from the latest snapshot in one workflow; raw paths never cross into Lua.
#[derive(Clone)]
pub(super) struct SnapshotIdentity {
    ids: BTreeSet<String>,
    snapshot_id: String,
}

pub(super) type GitSnapshotState = Rc<RefCell<Option<SnapshotIdentity>>>;

pub(super) fn build_git(
    lua: &Lua,
    in_tool: Rc<Cell<bool>>,
    pending: Pending,
    snapshot: GitSnapshotState,
) -> Result<Table> {
    let git = lua
        .create_table()
        .map_err(|cause| error::invalid(format!("cannot build koru.git: {cause}")))?;
    let flag = Rc::clone(&in_tool);
    let slot = Rc::clone(&pending);
    let snapshot_fn = lua
        .create_async_function(move |lua, ()| {
            let flag = Rc::clone(&flag);
            let slot = Rc::clone(&slot);
            async move {
                if flag.get() {
                    return Err(mlua::Error::RuntimeError(
                        "Git snapshots from a tool callback are not supported".to_owned(),
                    ));
                }
                *slot.borrow_mut() = Some(PendingRequest::GitSnapshot);
                let response: Table = lua.yield_with(Value::Nil).await?;
                if response.get::<bool>("ok")? {
                    Ok((Some(response.get::<Table>("snapshot")?), None::<Table>))
                } else {
                    Ok((None::<Table>, Some(response.get::<Table>("error")?)))
                }
            }
        })
        .map_err(|cause| error::invalid(format!("cannot build koru.git.snapshot: {cause}")))?;
    git.set("snapshot", snapshot_fn)
        .map_err(|cause| error::invalid(format!("cannot build koru.git: {cause}")))?;

    let flag = Rc::clone(&in_tool);
    let validate = lua
        .create_function(move |lua, proposal: Table| {
            if flag.get() {
                return Err(mlua::Error::RuntimeError(
                    "Git plan validation from a tool callback is not supported".to_owned(),
                ));
            }
            let known = snapshot
                .borrow()
                .clone()
                .ok_or_else(|| mlua::Error::RuntimeError("call koru.git.snapshot first".into()))?;
            let value =
                super::super::json::to_json(&Value::Table(proposal), &JsonLimits::default())
                    .map_err(|cause| mlua::Error::RuntimeError(cause.message().to_owned()))?;
            let plan = crate::git::validate_plan(&known.ids, &value)
                .map_err(|cause| mlua::Error::RuntimeError(cause.message().to_owned()))?;
            let JsonValue::Object(mut fields) = plan else {
                return Err(mlua::Error::RuntimeError(
                    "validated plan is not an object".into(),
                ));
            };
            fields.insert(
                "snapshot_id".to_owned(),
                JsonValue::String(known.snapshot_id.clone()),
            );
            let canonical = crate::json::emit(&JsonValue::Object(fields.clone()));
            let mut digest = Sha256::new();
            digest.update(known.snapshot_id.as_bytes());
            digest.update(canonical.as_bytes());
            fields.insert(
                "plan_id".to_owned(),
                JsonValue::String(format!("{:x}", digest.finalize())),
            );
            super::super::json::from_json(lua, &JsonValue::Object(fields))
                .map_err(|cause| mlua::Error::RuntimeError(cause.message().to_owned()))
        })
        .map_err(|cause| error::invalid(format!("cannot build koru.git.validate: {cause}")))?;
    git.set("validate", validate)
        .map_err(|cause| error::invalid(format!("cannot build koru.git: {cause}")))?;
    Ok(git)
}

pub(super) fn complete_snapshot(
    lua: &Lua,
    context: &ExecutionContext,
    approval: &mut dyn ApprovalProvider,
    slot: &GitSnapshotState,
) -> mlua::Result<Value> {
    *slot.borrow_mut() = None;
    let outcome = crate::git::capture(context, approval);
    let envelope = lua.create_table()?;
    match outcome {
        Ok(snapshot) => {
            let size: u64 = snapshot
                .changes
                .iter()
                .map(|change| (change.id.len() + change.kind.len() + change.excerpt.len()) as u64)
                .sum();
            context
                .reserve(
                    Resources {
                        bytes: size,
                        ..Resources::ZERO
                    },
                    std::time::Instant::now(),
                )
                .map_err(|cause| mlua::Error::RuntimeError(cause.message().to_owned()))?;
            let mut ids = BTreeSet::new();
            let changes = lua.create_table()?;
            for (index, change) in snapshot.changes.iter().enumerate() {
                ids.insert(change.id.clone());
                let item = lua.create_table()?;
                item.set("id", change.id.as_str())?;
                item.set("summary", change.kind.as_str())?;
                item.set("diff", change.excerpt.as_str())?;
                changes.raw_set(index + 1, item)?;
            }
            let snapshot_id = hex_digest(&snapshot.digest);
            *slot.borrow_mut() = Some(SnapshotIdentity {
                ids,
                snapshot_id: snapshot_id.clone(),
            });
            let value = lua.create_table()?;
            value.set("changes", changes)?;
            value.set("change_count", snapshot.changes.len())?;
            value.set("snapshot_id", snapshot_id)?;
            envelope.set("ok", true)?;
            envelope.set("snapshot", value)?;
        }
        Err(cause) => {
            envelope.set("ok", false)?;
            let error = lua.create_table()?;
            error.set("code", cause.code().as_str())?;
            error.set("message", cause.message())?;
            envelope.set("error", error)?;
        }
    }
    Ok(Value::Table(envelope))
}

fn hex_digest(digest: &[u8; 32]) -> String {
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}
