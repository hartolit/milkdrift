//! Dedicated bounded worker for the synchronous Hugging Face Hub adapter.

use std::num::NonZeroUsize;
use std::time::Duration;

use hf_hub_adapter::{
    HubClient, HubClientConfiguration, HubError, HubModelReference, ResolvedModelArtifacts,
};
use host_runtime::{
    BoundedReceiver, BoundedSender, ChannelCapacity, HostThread, ReceiveTimeoutError,
    SendTimeoutError, ThreadSpawnError, bounded, spawn_named,
};

use crate::{
    ApplicationConfigurationField, ApplicationError, ApplicationFailure, ApplicationFailureKind,
};

pub enum HubCommand {
    Resolve(HubModelReference),
    Shutdown,
}

pub enum HubEvent {
    Resolved(Result<ResolvedModelArtifacts, HubError>),
}

pub struct HubWorker {
    pub commands: BoundedSender<HubCommand>,
    pub events: BoundedReceiver<HubEvent>,
    pub thread: HostThread<()>,
}

pub fn start_hub_worker(
    configuration: HubClientConfiguration,
    channel_capacity: usize,
    worker_poll: Duration,
    event_send_timeout: Duration,
) -> Result<HubWorker, ApplicationError> {
    let client = HubClient::new(configuration)
        .map_err(|error| ApplicationFailure::new(ApplicationFailureKind::Hub, error))?;
    let capacity = NonZeroUsize::new(channel_capacity).ok_or(
        ApplicationError::InvalidConfiguration(ApplicationConfigurationField::HubChannelCapacity),
    )?;
    validate_duration(worker_poll, ApplicationConfigurationField::HubWorkerPoll)?;
    validate_duration(
        event_send_timeout,
        ApplicationConfigurationField::HubEventSendTimeout,
    )?;
    let (commands, command_receiver) = bounded(ChannelCapacity::new(capacity));
    let (events, event_receiver) = bounded(ChannelCapacity::new(capacity));
    let thread = spawn_named("llm-hub-resolver", move || {
        run_hub_worker(
            &client,
            &command_receiver,
            &events,
            worker_poll,
            event_send_timeout,
        );
    })
    .map_err(worker_spawn_failure)?;
    Ok(HubWorker {
        commands,
        events: event_receiver,
        thread,
    })
}

fn run_hub_worker(
    client: &HubClient,
    commands: &BoundedReceiver<HubCommand>,
    events: &BoundedSender<HubEvent>,
    worker_poll: Duration,
    event_send_timeout: Duration,
) {
    loop {
        match commands.receive_timeout(worker_poll) {
            Ok(HubCommand::Resolve(reference)) => {
                let mut pending = HubEvent::Resolved(client.resolve_llama(&reference));
                loop {
                    match events.send_timeout(pending, event_send_timeout) {
                        Ok(()) => break,
                        Err(SendTimeoutError::Timeout(event)) => pending = event,
                        Err(SendTimeoutError::Disconnected(_)) => return,
                    }
                }
            }
            Ok(HubCommand::Shutdown) | Err(ReceiveTimeoutError::Disconnected) => return,
            Err(ReceiveTimeoutError::Timeout) => {}
        }
    }
}

fn worker_spawn_failure(error: ThreadSpawnError) -> ApplicationError {
    ApplicationFailure::new(ApplicationFailureKind::Worker, error).into()
}

const fn validate_duration(
    duration: Duration,
    field: ApplicationConfigurationField,
) -> Result<(), ApplicationError> {
    if duration.is_zero() {
        Err(ApplicationError::InvalidConfiguration(field))
    } else {
        Ok(())
    }
}
