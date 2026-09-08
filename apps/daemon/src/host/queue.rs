//! Serialize durable calls while leaving sockets and external effects on their own workers.
//!
//! Ordinary queue saturation returns overload. Once queued, a call can still execute after
//! its caller loses the reply or times out, which is why mutations need durable receipts.
//! A panicking request closes ordinary admission but leaves final persistence and shutdown
//! calls serviceable so workers can relinquish their owned work.

use std::{
    sync::Arc, sync::Weak, sync::mpsc::SyncSender, sync::mpsc::TrySendError,
    sync::mpsc::sync_channel, thread,
};

use super::{OWNER_RESPONSE_TIMEOUT, health::SharedHealth};

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

use super::{DaemonHost, EffectShutdownOutcome, Owner, PublicFailure, health::QueuedRequestGuard};
use milkdrift_control_protocol::ErrorCode;
use std::{sync::mpsc::Receiver, sync::mpsc::RecvTimeoutError, time::Duration};
use tokio::sync::oneshot;

type OwnerTask = Box<dyn FnOnce(&mut Owner) + Send + 'static>;

pub(super) struct OwnerRequest {
    pub(super) execute: OwnerTask,
    pub(super) stop_owner: bool,
    pub(super) queued: Option<QueuedRequestGuard>,
}

impl OwnerRequest {
    pub(super) fn mark_queued(&mut self, health: &Arc<SharedHealth>) {
        self.queued = Some(health.track_queued_request());
    }

    pub(super) fn mark_dequeued(&mut self) {
        if let Some(queued) = self.queued.take() {
            queued.release();
        }
    }
}

impl DaemonHost {
    pub(super) async fn dispatch<T>(
        &self,
        shutdown: bool,
        operation: impl FnOnce(&mut Owner) -> Result<T, PublicFailure> + Send + 'static,
    ) -> Result<T, PublicFailure>
    where
        T: Send + 'static,
    {
        self.dispatch_inner(shutdown, !shutdown, shutdown, operation)
            .await
    }

    pub(super) async fn dispatch_draining<T>(
        &self,
        operation: impl FnOnce(&mut Owner) -> Result<T, PublicFailure> + Send + 'static,
    ) -> Result<T, PublicFailure>
    where
        T: Send + 'static,
    {
        self.dispatch_inner(false, false, true, operation).await
    }

    pub(super) async fn dispatch_inner<T>(
        &self,
        stop_owner: bool,
        require_ready: bool,
        use_shutdown_deadline: bool,
        operation: impl FnOnce(&mut Owner) -> Result<T, PublicFailure> + Send + 'static,
    ) -> Result<T, PublicFailure>
    where
        T: Send + 'static,
    {
        if require_ready && !self.health.is_ready() {
            return Err(PublicFailure::new(
                ErrorCode::Unavailable,
                "daemon is not ready",
                true,
            ));
        }
        let started = tokio::time::Instant::now();
        let (reply, receiver) = oneshot::channel();
        let request_health = self.health.clone();
        let mut pending = OwnerRequest {
            execute: Box::new(move |owner| {
                if require_ready && !request_health.is_ready() {
                    let _ = reply.send(Err(PublicFailure::new(
                        ErrorCode::Unavailable,
                        "daemon is not ready",
                        true,
                    )));
                    return;
                }
                let _ = reply.send(operation(owner));
            }),
            stop_owner,
            queued: None,
        };
        loop {
            pending.mark_queued(&self.health);
            match self.sender.try_send(pending) {
                Ok(()) => break,
                Err(TrySendError::Full(mut returned)) if use_shutdown_deadline => {
                    returned.mark_dequeued();
                    if started.elapsed() >= self.shutdown_deadline {
                        return Err(PublicFailure::new(
                            ErrorCode::Timeout,
                            "runtime owner shutdown could not enter the bounded queue before its deadline",
                            true,
                        ));
                    }
                    pending = returned;
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }
                Err(TrySendError::Full(_)) => {
                    return Err(PublicFailure::new(
                        ErrorCode::Overload,
                        "runtime owner request queue is full",
                        true,
                    ));
                }
                Err(TrySendError::Disconnected(_)) => {
                    return Err(PublicFailure::new(
                        ErrorCode::Unavailable,
                        "runtime owner is unavailable",
                        true,
                    ));
                }
            }
        }
        let response_timeout = if use_shutdown_deadline {
            self.shutdown_deadline.saturating_sub(started.elapsed())
        } else {
            OWNER_RESPONSE_TIMEOUT
        };
        if response_timeout.is_zero() {
            return Err(PublicFailure::new(
                ErrorCode::Timeout,
                "runtime owner shutdown response deadline elapsed",
                true,
            ));
        }
        match tokio::time::timeout(response_timeout, receiver).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err(PublicFailure::new(
                ErrorCode::Unavailable,
                "runtime owner response channel closed",
                true,
            )),
            Err(_) => Err(PublicFailure::new(
                ErrorCode::Timeout,
                "runtime owner response deadline elapsed",
                true,
            )),
        }
    }
}

impl Owner {
    pub(super) fn run(
        &mut self,
        receiver: Receiver<OwnerRequest>,
        maintenance: Duration,
        health: &SharedHealth,
    ) {
        loop {
            match receiver.recv_timeout(maintenance) {
                Ok(mut request) => {
                    request.mark_dequeued();
                    let stop_owner = request.stop_owner;
                    if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        (request.execute)(self);
                    }))
                    .is_err()
                    {
                        self.request_panicked = true;
                        self.runtime.begin_shutdown();
                        health.failure("runtime owner request panicked");
                        health.set_lifecycle(super::health::Lifecycle::Failed);
                        // Keep servicing final worker persistence and shutdown requests.
                        continue;
                    }
                    if stop_owner {
                        return;
                    }
                    if !self.request_panicked {
                        self.maintenance(health);
                    }
                }
                Err(RecvTimeoutError::Timeout) if !self.request_panicked => {
                    self.maintenance(health)
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => {
                    let _ = self.shutdown(health, None, EffectShutdownOutcome::failed());
                    return;
                }
            }
        }
    }
}
