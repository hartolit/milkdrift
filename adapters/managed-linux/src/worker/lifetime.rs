//! Creator ownership and durable generation entry share one lifetime across nested model calls.
use super::{ManagedWorkerAdapter, adapter_failure};
use crate::{LinuxManagedPlatform, command, platform_error};
use milkdrift_capability_host::{AdapterError, AdapterInvocation, managed::ManagedError};
use milkdrift_persistence::managed::{ApprovedSetup, ManagedUse};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, Mutex, atomic::AtomicBool},
};

impl ManagedWorkerAdapter {
    pub(super) fn enter_use(
        &self,
        invocation: &AdapterInvocation<'_>,
        id: &str,
        physical_name: &str,
    ) -> Result<(ManagedUse, bool), AdapterError> {
        let nested = invocation.context().and_then(|context| {
            context
                .model_tool_owner()
                .filter(|(parent, _)| *parent != invocation.request().invocation())
                .map(|(parent, maximum)| (context, parent, maximum))
        });
        let usage = if let Some((context, parent, maximum)) = nested {
            if !(1..=maximum).any(|call| {
                milkdrift_persistence::managed::task_tool_invocation(parent, call)
                    .is_ok_and(|expected| &expected == invocation.request().invocation())
            }) {
                return Err(AdapterError::rejected(
                    "managed tool call is outside its frozen parent allowance",
                ));
            }
            let workflow_id = context.task_workflow_id()?;
            self.store
                .reserve_managed_task_tool(
                    &workflow_id,
                    parent,
                    invocation.request(),
                    invocation.resolution(),
                )
                .map_err(adapter_failure)?
        } else {
            self.store
                .managed_use(id)
                .map_err(adapter_failure)?
                .ok_or_else(|| AdapterError::rejected("worker lacks durable generation hold"))?
        };
        self.store
            .enter_managed_use(id, usage.claim, physical_name)
            .map_err(adapter_failure)?;
        Ok((usage, nested.is_some()))
    }
}

pub(super) struct OwnedTask {
    pub(super) platform: Arc<LinuxManagedPlatform>,
    pub(super) id: String,
    pub(super) name: String,
    pub(super) active: Arc<Mutex<BTreeSet<String>>>,
}
impl Drop for OwnedTask {
    fn drop(&mut self) {
        // This guard proves only that the creator has returned. Durable use remains held
        // until explicit cleanup or the recovery fence proves physical absence. Recover
        // poisoned locks solely to remove this creator, so recovery cannot wait forever.
        let mut active = self
            .platform
            .active
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let mut mine = self.active.lock().unwrap_or_else(|e| e.into_inner());
        active.remove(&self.id);
        mine.remove(&self.id);
        self.platform.quiescent.notify_all();
    }
}

pub(super) fn register_creator(
    platform: &Mutex<BTreeMap<String, Arc<AtomicBool>>>,
    adapter: &Mutex<BTreeSet<String>>,
    id: &str,
    cancel: Arc<AtomicBool>,
) -> Result<(), AdapterError> {
    let mut active = platform
        .lock()
        .map_err(|_| AdapterError::unavailable("worker ownership unavailable"))?;
    let mut mine = adapter
        .lock()
        .map_err(|_| AdapterError::unavailable("worker ownership unavailable"))?;
    if active.len() >= milkdrift_capability::managed::MAX_MANAGED_USES
        || active.contains_key(id)
        || mine.contains(id)
    {
        return Err(AdapterError::rejected("duplicate or excess active worker"));
    }
    // Acquire both fallible locks before publishing either view of creator ownership.
    active.insert(id.to_owned(), cancel);
    mine.insert(id.to_owned());
    Ok(())
}

pub(super) struct StoppedTask {
    pub(super) output: Result<command::CommandOutput, ManagedError>,
}

pub(super) fn run_and_remove_task(
    run: impl FnOnce() -> Result<command::CommandOutput, ManagedError>,
    remove: impl FnOnce() -> Result<(), ManagedError>,
) -> Result<StoppedTask, ManagedError> {
    let output = std::panic::catch_unwind(std::panic::AssertUnwindSafe(run))
        .unwrap_or_else(|_| Err(platform_error("worker observation panicked")));
    if let Err(error) = remove() {
        let original = output
            .as_ref()
            .err()
            .map_or_else(|| "worker completed".to_owned(), ToString::to_string);
        return Err(platform_error(format!(
            "worker cleanup unconfirmed: {}; original: {}",
            milkdrift_contracts::truncate_utf8(&error.to_string(), 220),
            milkdrift_contracts::truncate_utf8(&original, 220),
        )));
    }
    Ok(StoppedTask { output })
}

pub(super) fn task_identity<'a>(
    value: &'a serde_json::Value,
    setup: &ApprovedSetup,
    name: &str,
) -> Result<&'a str, ManagedError> {
    LinuxManagedPlatform::verify_labels(value, setup, Some(&setup.recipe.digest))?;
    let identity = value.get("Id").and_then(|v| v.as_str()).unwrap_or("");
    if value.get("Name").and_then(|v| v.as_str()) != Some(name)
        || identity.len() != 64
        || !identity
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(platform_error("task container identity differs"));
    }
    // Names can be reused between inspection and an engine command. Address only the inspected
    // container so another creator's replacement cannot be started or removed by this task.
    Ok(identity)
}
