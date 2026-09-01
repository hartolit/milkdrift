//! Bounded synchronous bridge into the daemon's single durable owner thread.

use std::{
    sync::{
        Arc, Weak,
        mpsc::{SyncSender, TrySendError, sync_channel},
    },
    thread,
};

use super::{OWNER_RESPONSE_TIMEOUT, OwnerRequest, SharedHealth};

#[derive(Clone)]
pub(super) struct OwnerQueue {
    sender: Weak<SyncSender<OwnerRequest>>,
    health: Arc<SharedHealth>,
    owner_thread: thread::ThreadId,
}

impl OwnerQueue {
    pub(super) fn new(
        sender: Weak<SyncSender<OwnerRequest>>,
        health: Arc<SharedHealth>,
        owner_thread: thread::ThreadId,
    ) -> Self {
        Self {
            sender,
            health,
            owner_thread,
        }
    }

    pub(super) fn call<T, E>(
        &self,
        operation: impl FnOnce() -> Result<T, E> + Send + 'static,
        map_failure: fn(OwnerCallFailure) -> E,
    ) -> Result<T, E>
    where
        T: Send + 'static,
        E: Send + 'static,
    {
        if thread::current().id() == self.owner_thread {
            return operation();
        }

        let (reply, receiver) = sync_channel(1);
        let owner_thread = self.owner_thread;
        let mut request = OwnerRequest {
            execute: Box::new(move |_| {
                assert_eq!(
                    thread::current().id(),
                    owner_thread,
                    "durable operation escaped the daemon owner thread"
                );
                let _ = reply.send(operation());
            }),
            stop_owner: false,
            queued: None,
        };
        request.mark_queued(&self.health);
        let Some(sender) = self.sender.upgrade() else {
            return Err(map_failure(OwnerCallFailure::Disconnected));
        };
        match sender.try_send(request) {
            Ok(()) => {}
            Err(TrySendError::Full(_)) => {
                return Err(map_failure(OwnerCallFailure::QueueFull));
            }
            Err(TrySendError::Disconnected(_)) => {
                return Err(map_failure(OwnerCallFailure::Disconnected));
            }
        }
        match receiver.recv_timeout(OWNER_RESPONSE_TIMEOUT) {
            Ok(result) => result,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                Err(map_failure(OwnerCallFailure::ResponseTimeout))
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                Err(map_failure(OwnerCallFailure::Disconnected))
            }
        }
    }
}

#[derive(Clone, Copy)]
pub(super) enum OwnerCallFailure {
    QueueFull,
    Disconnected,
    ResponseTimeout,
}

impl OwnerCallFailure {
    pub(super) const fn message(self) -> &'static str {
        match self {
            Self::QueueFull => "daemon runtime owner queue is full",
            Self::Disconnected => "daemon runtime owner is unavailable",
            Self::ResponseTimeout => "daemon runtime owner response deadline elapsed",
        }
    }
}
