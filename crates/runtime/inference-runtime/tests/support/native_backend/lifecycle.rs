use super::{
    CandleRuntime, CommandTicket, EVENT_TIMEOUT, MemoryFootprint, ModelHandle, RuntimeCommand,
    RuntimeEvent, RuntimeThread, TestResult, UnloadPolicy, UnloadStatus,
};

pub(crate) const UNLOAD_TICKET: CommandTicket = CommandTicket::new(90);
pub(crate) const UNLOADED_SNAPSHOT_TICKET: CommandTicket = CommandTicket::new(91);
pub(crate) const SHUTDOWN_TICKET: CommandTicket = CommandTicket::new(92);
pub(crate) fn unload_model(hosted: &CandleRuntime, handle: ModelHandle) -> TestResult {
    hosted
        .try_submit(RuntimeCommand::UnloadModel {
            ticket: UNLOAD_TICKET,
            handle,
            policy: UnloadPolicy::RejectIfBusy,
        })
        .map_err(|error| format!("unload command rejected: {error:?}"))?;
    match hosted
        .receive_timeout(EVENT_TIMEOUT)
        .map_err(|error| format!("unload event failed: {error:?}"))?
    {
        RuntimeEvent::ModelUnload {
            ticket,
            result: Ok(receipt),
        } if ticket == UNLOAD_TICKET && receipt.status == UnloadStatus::Unloaded => {
            assert_eq!(receipt.handle, handle);
            assert_eq!(receipt.cancelled_requests, 0);
            assert_unloaded_snapshot(hosted)
        }
        RuntimeEvent::ModelUnload {
            result: Err(error), ..
        } => Err(format!("model unload failed: {error:?}")),
        event => Err(format!(
            "unexpected unload event for ticket {:?}",
            event.ticket()
        )),
    }
}

pub(crate) fn assert_unloaded_snapshot(hosted: &CandleRuntime) -> TestResult {
    hosted
        .try_submit(RuntimeCommand::Snapshot {
            ticket: UNLOADED_SNAPSHOT_TICKET,
        })
        .map_err(|error| format!("post-unload snapshot command rejected: {error:?}"))?;
    match hosted
        .receive_timeout(EVENT_TIMEOUT)
        .map_err(|error| format!("post-unload snapshot event failed: {error:?}"))?
    {
        RuntimeEvent::Snapshot {
            ticket,
            runtime,
            models,
            retained_models,
        } if ticket == UNLOADED_SNAPSHOT_TICKET => {
            assert_eq!(runtime.loaded_models, 0);
            assert_eq!(runtime.active_requests, 0);
            assert_eq!(runtime.reserved_footprint, MemoryFootprint::default());
            assert!(runtime.unverified_ownership.is_none());
            assert!(!runtime.admission_blocked);
            assert_eq!(runtime.generation_workspaces, 0);
            assert_eq!(
                runtime.reserved_generation_workspace,
                MemoryFootprint::default()
            );
            assert_eq!(runtime.pending_cleanup_models, 0);
            assert_eq!(runtime.pending_cleanup_sequences, 0);
            assert_eq!(runtime.exhausted_cleanup_models, 0);
            assert_eq!(runtime.exhausted_cleanup_sequences, 0);
            assert!(runtime.maintenance_error.is_none());
            assert!(models.is_empty());
            assert!(retained_models.is_empty());
            Ok(())
        }
        event => Err(format!(
            "unexpected post-unload snapshot event for ticket {:?}",
            event.ticket()
        )),
    }
}

pub(crate) fn shutdown(hosted: CandleRuntime, thread: RuntimeThread) -> TestResult {
    hosted
        .try_submit(RuntimeCommand::Shutdown {
            ticket: SHUTDOWN_TICKET,
        })
        .map_err(|error| format!("shutdown command rejected: {error:?}"))?;
    match hosted
        .receive_timeout(EVENT_TIMEOUT)
        .map_err(|error| format!("shutdown event failed: {error:?}"))?
    {
        RuntimeEvent::Shutdown {
            ticket,
            result: Ok(receipt),
        } if ticket == SHUTDOWN_TICKET => {
            assert_eq!(receipt.unloaded_models, 0);
            assert_eq!(receipt.cancelled_requests, 0);
        }
        RuntimeEvent::Shutdown {
            result: Err(error), ..
        } => return Err(format!("runtime shutdown failed: {error:?}")),
        event => {
            return Err(format!(
                "unexpected shutdown event for ticket {:?}",
                event.ticket()
            ));
        }
    }
    thread.join().map_err(|error| error.to_string())
}
