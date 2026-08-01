//! Bounded cooperative shutdown for application-owned host workers.
//!
//! Frontends must call [`ApplicationRuntime::shutdown`] on normal closure; dropping the runtime
//! intentionally does not perform an unbounded worker join.

use std::time::{Duration, Instant};

use host_runtime::{HostThread, SendTimeoutError, yield_now};
use inference_runtime::{
    CommandTicket, HostedRuntime, RuntimeCommand, RuntimeError, RuntimeEvent, RuntimeThread,
    ShutdownReceipt,
};

use crate::hub_worker::HubCommand;
use crate::local::LocalInference;
use crate::support::thread_failure;
use crate::{
    ApplicationError, ApplicationFailure, ApplicationFailureKind, ApplicationRuntime,
    ApplicationTiming, ApplicationWorker,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShutdownStatus {
    Running,
    Stopping,
    Stopped,
    RetryableFailure,
    TerminalFailure,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum InferenceTerminalFailure {
    Runtime(RuntimeError),
    EndpointDisconnected,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum InferenceShutdownState {
    Running,
    Awaiting(CommandTicket),
    CleanlyStopped,
    TerminalFailure(InferenceTerminalFailure),
}

pub struct ShutdownControl {
    pub(crate) status: ShutdownStatus,
    hub_stop_requested: bool,
    inference: InferenceShutdownState,
    #[cfg(test)]
    pub(crate) forced_runtime_shutdown_failure: Option<RuntimeError>,
    #[cfg(test)]
    pub(crate) forced_runtime_join_timeouts: usize,
    #[cfg(test)]
    pub(crate) forced_hub_join_timeouts: usize,
}

impl Default for ShutdownControl {
    fn default() -> Self {
        Self {
            status: ShutdownStatus::Running,
            hub_stop_requested: false,
            inference: InferenceShutdownState::Running,
            #[cfg(test)]
            forced_runtime_shutdown_failure: None,
            #[cfg(test)]
            forced_runtime_join_timeouts: 0,
            #[cfg(test)]
            forced_hub_join_timeouts: 0,
        }
    }
}

impl ShutdownControl {
    pub(crate) fn record_inference_disconnect(&mut self) {
        if !matches!(
            self.inference,
            InferenceShutdownState::CleanlyStopped | InferenceShutdownState::TerminalFailure(_)
        ) {
            self.inference = InferenceShutdownState::TerminalFailure(
                InferenceTerminalFailure::EndpointDisconnected,
            );
            self.status = ShutdownStatus::TerminalFailure;
        }
    }

    fn record_inference_failure(&mut self, error: RuntimeError) {
        if !matches!(self.inference, InferenceShutdownState::TerminalFailure(_)) {
            self.inference =
                InferenceShutdownState::TerminalFailure(InferenceTerminalFailure::Runtime(error));
            self.status = ShutdownStatus::TerminalFailure;
        }
    }

    const fn terminal_failure(&self) -> Option<InferenceTerminalFailure> {
        match self.inference {
            InferenceShutdownState::TerminalFailure(failure) => Some(failure),
            InferenceShutdownState::Running
            | InferenceShutdownState::Awaiting(_)
            | InferenceShutdownState::CleanlyStopped => None,
        }
    }

    const fn inference_cleanly_stopped(&self) -> bool {
        matches!(self.inference, InferenceShutdownState::CleanlyStopped)
    }
}

pub fn shutdown(runtime: &mut ApplicationRuntime) -> Result<(), ApplicationError> {
    if runtime.shutdown_control.status == ShutdownStatus::Stopped {
        return Ok(());
    }

    runtime.shutdown_control.status = ShutdownStatus::Stopping;
    runtime.state.begin_shutdown();

    let mut first_error = request_hub_shutdown(runtime).err();
    record_first_error(&mut first_error, shutdown_runtime(runtime).err());
    record_first_error(&mut first_error, join_runtime_worker(runtime).err());
    record_first_error(&mut first_error, join_hub_worker(runtime).err());

    let workers_stopped = workers_confirmed_stopped(runtime);
    if workers_stopped {
        runtime.release_incompatible_model_cleanup();
    }
    if let Some(failure) = runtime.shutdown_control.terminal_failure() {
        runtime.shutdown_control.status = ShutdownStatus::TerminalFailure;
        return Err(terminal_inference_error(failure));
    }
    match first_error {
        None if workers_stopped && runtime.shutdown_control.inference_cleanly_stopped() => {
            runtime.shutdown_control.status = ShutdownStatus::Stopped;
            Ok(())
        }
        Some(error) => {
            runtime.shutdown_control.status = ShutdownStatus::RetryableFailure;
            Err(error)
        }
        None => {
            runtime.shutdown_control.status = ShutdownStatus::RetryableFailure;
            let worker = if runtime.local.thread_is_present() {
                ApplicationWorker::Inference
            } else {
                ApplicationWorker::Hub
            };
            Err(ApplicationError::ShutdownTimeout(worker))
        }
    }
}

const fn workers_confirmed_stopped(runtime: &ApplicationRuntime) -> bool {
    !runtime.local.thread_is_present() && runtime.hub_thread.is_none()
}

fn request_hub_shutdown(runtime: &mut ApplicationRuntime) -> Result<(), ApplicationError> {
    if runtime.hub_thread.is_none() || runtime.shutdown_control.hub_stop_requested {
        runtime.state.disconnect_hub();
        return Ok(());
    }
    if !runtime.state.hub_available() {
        runtime.shutdown_control.hub_stop_requested = true;
        return Ok(());
    }
    match runtime.hub_commands.send_timeout(
        HubCommand::Shutdown,
        runtime.configuration.timing.hub_command_shutdown_timeout,
    ) {
        Ok(()) | Err(SendTimeoutError::Disconnected(_)) => {
            runtime.shutdown_control.hub_stop_requested = true;
            runtime.state.disconnect_hub();
            Ok(())
        }
        Err(SendTimeoutError::Timeout(_)) => Err(ApplicationError::HubBusy),
    }
}

fn shutdown_runtime(runtime: &mut ApplicationRuntime) -> Result<(), ApplicationError> {
    let deadline = checked_deadline(
        runtime.configuration.timing.runtime_shutdown_timeout,
        crate::ApplicationConfigurationField::RuntimeShutdownTimeout,
    )?;
    let ticket = match runtime.shutdown_control.inference {
        InferenceShutdownState::Running => {
            if !runtime.local.thread_is_present() {
                runtime.shutdown_control.record_inference_disconnect();
                runtime.state.disconnect_inference();
                runtime.release_incompatible_model_cleanup();
                return Err(ApplicationError::RuntimeDisconnected);
            }
            while runtime.local.runtime().try_receive().is_ok() {}
            let ticket = runtime.next_ticket()?;
            match request_runtime_shutdown_until(runtime.local.runtime(), ticket, deadline)? {
                RuntimeShutdownRequest::Disconnected => {
                    runtime.shutdown_control.record_inference_disconnect();
                    runtime.state.disconnect_inference();
                    runtime.release_incompatible_model_cleanup();
                    return Err(ApplicationError::RuntimeDisconnected);
                }
                RuntimeShutdownRequest::Submitted => {
                    runtime.shutdown_control.inference = InferenceShutdownState::Awaiting(ticket);
                    runtime.state.disconnect_inference();
                    ticket
                }
            }
        }
        InferenceShutdownState::Awaiting(ticket) => ticket,
        InferenceShutdownState::CleanlyStopped => return Ok(()),
        InferenceShutdownState::TerminalFailure(failure) => {
            return Err(terminal_inference_error(failure));
        }
    };

    let outcome = await_runtime_shutdown_until(
        runtime.local.runtime(),
        ticket,
        deadline,
        runtime.configuration.timing.runtime_shutdown_event_poll,
    )?;
    #[cfg(test)]
    let outcome = runtime
        .shutdown_control
        .forced_runtime_shutdown_failure
        .take()
        .map_or(outcome, |error| RuntimeShutdown::Finished(Err(error)));
    match outcome {
        RuntimeShutdown::Disconnected => {
            runtime.shutdown_control.record_inference_disconnect();
            runtime.release_incompatible_model_cleanup();
            Err(ApplicationError::RuntimeDisconnected)
        }
        RuntimeShutdown::Finished(Ok(_)) => {
            runtime.shutdown_control.inference = InferenceShutdownState::CleanlyStopped;
            Ok(())
        }
        RuntimeShutdown::Finished(Err(error)) => {
            runtime.shutdown_control.record_inference_failure(error);
            Err(inference_shutdown_error(error))
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RuntimeShutdownRequest {
    Disconnected,
    Submitted,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RuntimeShutdown {
    Disconnected,
    Finished(Result<ShutdownReceipt, RuntimeError>),
}

fn shutdown_runtime_worker<S>(
    runtime: &HostedRuntime<S>,
    ticket: CommandTicket,
    timeout: Duration,
    event_poll: Duration,
) -> Result<RuntimeShutdown, ApplicationError> {
    while runtime.try_receive().is_ok() {}
    let deadline = checked_deadline(
        timeout,
        crate::ApplicationConfigurationField::RuntimeShutdownTimeout,
    )?;
    match request_runtime_shutdown_until(runtime, ticket, deadline)? {
        RuntimeShutdownRequest::Disconnected => Ok(RuntimeShutdown::Disconnected),
        RuntimeShutdownRequest::Submitted => {
            await_runtime_shutdown_until(runtime, ticket, deadline, event_poll)
        }
    }
}

fn request_runtime_shutdown_until<S>(
    runtime: &HostedRuntime<S>,
    ticket: CommandTicket,
    deadline: Instant,
) -> Result<RuntimeShutdownRequest, ApplicationError> {
    let mut pending = RuntimeCommand::Shutdown { ticket };
    loop {
        match runtime.try_submit(pending) {
            Ok(()) => return Ok(RuntimeShutdownRequest::Submitted),
            Err(inference_runtime::RuntimeSubmitError::Disconnected(_)) => {
                return Ok(RuntimeShutdownRequest::Disconnected);
            }
            Err(inference_runtime::RuntimeSubmitError::Full(command)) => {
                if remaining_until(deadline).is_none() {
                    return Err(ApplicationError::RuntimeBusy);
                }
                pending = command;
                yield_now();
            }
        }
    }
}

fn await_runtime_shutdown_until<S>(
    runtime: &HostedRuntime<S>,
    ticket: CommandTicket,
    deadline: Instant,
    event_poll: Duration,
) -> Result<RuntimeShutdown, ApplicationError> {
    loop {
        let remaining = remaining_until(deadline).ok_or(ApplicationError::ShutdownTimeout(
            ApplicationWorker::Inference,
        ))?;
        match runtime.receive_timeout(event_poll.min(remaining)) {
            Ok(RuntimeEvent::Shutdown {
                ticket: event_ticket,
                result,
            }) if event_ticket == ticket => return Ok(RuntimeShutdown::Finished(result)),
            Ok(_) | Err(inference_runtime::RuntimeReceiveError::Timeout) => {}
            Err(inference_runtime::RuntimeReceiveError::Disconnected) => {
                return Ok(RuntimeShutdown::Disconnected);
            }
        }
    }
}

fn normalize_runtime_shutdown(outcome: RuntimeShutdown) -> Result<(), ApplicationError> {
    match outcome {
        RuntimeShutdown::Disconnected => Err(ApplicationError::RuntimeDisconnected),
        RuntimeShutdown::Finished(result) => result.map(|_| ()).map_err(inference_shutdown_error),
    }
}

fn terminal_inference_error(failure: InferenceTerminalFailure) -> ApplicationError {
    match failure {
        InferenceTerminalFailure::Runtime(error) => inference_shutdown_error(error),
        InferenceTerminalFailure::EndpointDisconnected => ApplicationError::RuntimeDisconnected,
    }
}

fn inference_shutdown_error(error: RuntimeError) -> ApplicationError {
    ApplicationFailure::from_debug(
        ApplicationFailureKind::Inference,
        "inference shutdown failed",
        error,
    )
    .into()
}

fn join_runtime_worker(runtime: &mut ApplicationRuntime) -> Result<(), ApplicationError> {
    if !runtime.local.thread_is_present() {
        return Ok(());
    }
    #[cfg(test)]
    if runtime.shutdown_control.forced_runtime_join_timeouts > 0 {
        runtime.shutdown_control.forced_runtime_join_timeouts -= 1;
        return Err(ApplicationError::ShutdownTimeout(
            ApplicationWorker::Inference,
        ));
    }

    let timeout = runtime.configuration.timing.runtime_join_timeout;
    let poll = runtime.configuration.timing.runtime_join_poll;
    let result = finish_runtime_thread(runtime.local.thread_slot(), timeout, poll);
    if !runtime.local.thread_is_present() {
        runtime.state.disconnect_inference();
        runtime.release_incompatible_model_cleanup();
    }
    result
}

fn finish_runtime_thread(
    thread: &mut Option<RuntimeThread>,
    timeout: Duration,
    poll: Duration,
) -> Result<(), ApplicationError> {
    let Some(pending) = thread.as_ref() else {
        return Ok(());
    };
    wait_for_runtime_thread(pending, timeout, poll)?;
    let Some(finished) = thread.take() else {
        return Ok(());
    };
    finished.join().map_err(thread_failure)
}

fn join_hub_worker(runtime: &mut ApplicationRuntime) -> Result<(), ApplicationError> {
    if runtime.hub_thread.is_none() {
        return Ok(());
    }
    #[cfg(test)]
    if runtime.shutdown_control.forced_hub_join_timeouts > 0 {
        runtime.shutdown_control.forced_hub_join_timeouts -= 1;
        return Err(ApplicationError::ShutdownTimeout(ApplicationWorker::Hub));
    }

    let result = finish_host_thread(
        &mut runtime.hub_thread,
        runtime.configuration.timing.hub_shutdown_timeout,
        runtime.configuration.timing.hub_shutdown_poll,
    );
    if runtime.hub_thread.is_none() {
        runtime.shutdown_control.hub_stop_requested = true;
        runtime.state.disconnect_hub();
    }
    result
}

fn finish_host_thread(
    thread: &mut Option<HostThread<()>>,
    timeout: Duration,
    poll: Duration,
) -> Result<(), ApplicationError> {
    let Some(pending) = thread.as_ref() else {
        return Ok(());
    };
    wait_for_host_thread(pending, timeout, poll)?;
    let Some(finished) = thread.take() else {
        return Ok(());
    };
    finished.join().map_err(thread_failure)
}

pub fn rollback_started_inference(
    local: &mut LocalInference,
    timing: ApplicationTiming,
) -> Result<(), ApplicationError> {
    let mut first_error = shutdown_runtime_worker(
        local.runtime(),
        CommandTicket::new(1),
        timing.runtime_shutdown_timeout,
        timing.runtime_shutdown_event_poll,
    )
    .and_then(normalize_runtime_shutdown)
    .err();
    record_first_error(
        &mut first_error,
        finish_runtime_thread(
            local.thread_slot(),
            timing.runtime_join_timeout,
            timing.runtime_join_poll,
        )
        .err(),
    );
    first_error.map_or(Ok(()), Err)
}

fn wait_for_runtime_thread(
    thread: &RuntimeThread,
    timeout: Duration,
    poll: Duration,
) -> Result<(), ApplicationError> {
    let deadline = checked_deadline(
        timeout,
        crate::ApplicationConfigurationField::RuntimeJoinTimeout,
    )?;
    while !thread.is_finished() {
        let remaining = remaining_until(deadline).ok_or(ApplicationError::ShutdownTimeout(
            ApplicationWorker::Inference,
        ))?;
        std::thread::sleep(poll.min(remaining));
    }
    Ok(())
}

fn wait_for_host_thread(
    thread: &HostThread<()>,
    timeout: Duration,
    poll: Duration,
) -> Result<(), ApplicationError> {
    let deadline = checked_deadline(
        timeout,
        crate::ApplicationConfigurationField::HubShutdownTimeout,
    )?;
    while !thread.is_finished() {
        let remaining = remaining_until(deadline)
            .ok_or(ApplicationError::ShutdownTimeout(ApplicationWorker::Hub))?;
        std::thread::sleep(poll.min(remaining));
    }
    Ok(())
}

fn checked_deadline(
    timeout: Duration,
    field: crate::ApplicationConfigurationField,
) -> Result<Instant, ApplicationError> {
    Instant::now()
        .checked_add(timeout)
        .ok_or(ApplicationError::InvalidConfiguration(field))
}

fn remaining_until(deadline: Instant) -> Option<Duration> {
    deadline
        .checked_duration_since(Instant::now())
        .filter(|remaining| !remaining.is_zero())
}

fn record_first_error(first: &mut Option<ApplicationError>, candidate: Option<ApplicationError>) {
    if first.is_none() {
        *first = candidate;
    }
}

#[cfg(test)]
mod tests;
