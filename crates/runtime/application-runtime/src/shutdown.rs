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
use crate::support::thread_failure;
use crate::{
    ApplicationActivity, ApplicationError, ApplicationFailure, ApplicationFailureKind,
    ApplicationRuntime, ApplicationWorker,
};

pub fn shutdown(runtime: &mut ApplicationRuntime) -> Result<(), ApplicationError> {
    if runtime.state.activity() == ApplicationActivity::ShuttingDown {
        return Ok(());
    }
    runtime.state.begin_shutdown();

    let mut first_error = request_hub_shutdown(runtime).err();
    record_first_error(&mut first_error, shutdown_runtime(runtime).err());
    record_first_error(&mut first_error, join_runtime_worker(runtime).err());
    record_first_error(&mut first_error, join_hub_worker(runtime).err());

    first_error.map_or(Ok(()), Err)
}

fn request_hub_shutdown(runtime: &mut ApplicationRuntime) -> Result<(), ApplicationError> {
    if !runtime.state.hub_available() {
        return Ok(());
    }
    match runtime.hub_commands.send_timeout(
        HubCommand::Shutdown,
        runtime.configuration.timing.hub_command_shutdown_timeout,
    ) {
        Ok(()) | Err(SendTimeoutError::Disconnected(_)) => {
            runtime.state.disconnect_hub();
            Ok(())
        }
        Err(SendTimeoutError::Timeout(_)) => Err(ApplicationError::HubBusy),
    }
}

fn shutdown_runtime(runtime: &mut ApplicationRuntime) -> Result<(), ApplicationError> {
    if !runtime.state.inference_available() {
        return Ok(());
    }
    let ticket = runtime.next_ticket()?;
    let outcome = shutdown_runtime_worker(
        &runtime.inference,
        ticket,
        runtime.configuration.timing.runtime_shutdown_timeout,
        runtime.configuration.timing.runtime_shutdown_event_poll,
    )?;
    runtime.state.disconnect_inference();
    normalize_runtime_shutdown(outcome)
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
    let mut pending = RuntimeCommand::Shutdown { ticket };
    loop {
        match runtime.try_submit(pending) {
            Ok(()) => break,
            Err(inference_runtime::RuntimeSubmitError::Disconnected(_)) => {
                return Ok(RuntimeShutdown::Disconnected);
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
        RuntimeShutdown::Disconnected => Ok(()),
        RuntimeShutdown::Finished(result) => result.map(|_| ()).map_err(|error| {
            ApplicationFailure::from_debug(
                ApplicationFailureKind::Inference,
                "inference shutdown failed",
                error,
            )
            .into()
        }),
    }
}

fn join_runtime_worker(runtime: &mut ApplicationRuntime) -> Result<(), ApplicationError> {
    let Some(thread) = runtime.inference_thread.take() else {
        return Ok(());
    };
    wait_for_runtime_thread(
        &thread,
        runtime.configuration.timing.runtime_join_timeout,
        runtime.configuration.timing.runtime_join_poll,
    )?;
    thread.join().map_err(thread_failure)
}

fn join_hub_worker(runtime: &mut ApplicationRuntime) -> Result<(), ApplicationError> {
    let Some(thread) = runtime.hub_thread.take() else {
        return Ok(());
    };
    finish_host_thread(
        thread,
        runtime.configuration.timing.hub_shutdown_timeout,
        runtime.configuration.timing.hub_shutdown_poll,
    )
}

fn finish_host_thread(
    thread: HostThread<()>,
    timeout: Duration,
    poll: Duration,
) -> Result<(), ApplicationError> {
    wait_for_host_thread(&thread, timeout, poll)?;
    thread.join().map_err(thread_failure)
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
