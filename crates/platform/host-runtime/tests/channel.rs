//! Contract tests for bounded host infrastructure.

use std::num::NonZeroUsize;
use std::time::Duration;

use host_runtime::{ChannelCapacity, ReceiveTimeoutError, TrySendError, bounded, spawn_named};

#[test]
fn bounded_channel_reports_backpressure_without_losing_message() -> Result<(), String> {
    let capacity = NonZeroUsize::new(1).ok_or("non-zero test capacity")?;
    let (sender, receiver) = bounded(ChannelCapacity::new(capacity));
    sender.try_send(7_u32).map_err(|_| "first send failed")?;

    let retained = match sender.try_send(11_u32) {
        Err(TrySendError::Full(message)) => message,
        Err(TrySendError::Disconnected(_)) => return Err("channel disconnected".into()),
        Ok(()) => return Err("bounded channel accepted excess message".into()),
    };
    if retained != 11 {
        return Err("backpressure did not retain message ownership".into());
    }
    if receiver.try_receive().map_err(|_| "receive failed")? != 7 {
        return Err("unexpected first message".into());
    }
    Ok(())
}

#[test]
fn named_thread_and_receive_timeout_are_contained() -> Result<(), String> {
    let capacity = NonZeroUsize::new(1).ok_or("non-zero test capacity")?;
    let (sender, receiver) = bounded(ChannelCapacity::new(capacity));
    let thread = spawn_named("host-runtime-test", move || sender.try_send(23_u32))
        .map_err(|error| error.to_string())?;
    thread
        .join()
        .map_err(|error| error.to_string())?
        .map_err(|_| "thread send failed")?;

    let value = receiver
        .receive_timeout(Duration::from_millis(100))
        .map_err(|error| format!("receive failed: {error:?}"))?;
    if value != 23 {
        return Err("unexpected threaded message".into());
    }
    if receiver.receive_timeout(Duration::ZERO) != Err(ReceiveTimeoutError::Disconnected) {
        return Err("disconnected channel was not reported".into());
    }
    Ok(())
}
