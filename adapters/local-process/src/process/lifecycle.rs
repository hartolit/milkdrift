//! Retain the child, cancellation registration, and every started I/O worker until cleanup.

use std::{
    collections::BTreeMap,
    process::Child,
    sync::{
        Arc, Mutex,
        mpsc::{Receiver, sync_channel},
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};

use milkdrift_capability::InvocationId;
use milkdrift_capability_host::AdapterError;

use super::{
    monitor::{ProcessObservation, monitor_process},
    platform::{
        ActiveRegistration, ProcessControl, terminate_child_immediately,
        wait_for_owned_descendants_absence,
    },
    reporting::TerminalReportContext,
    streams::{Stream, StreamMessage, join_io, spawn_reader, spawn_stdin_writer},
};
use crate::config::ProcessProfile;

const STREAM_CHANNEL_MESSAGES: usize = 16;

pub(super) struct RunningProcess {
    child: Child,
    control: Arc<ProcessControl>,
    registration: Option<ActiveRegistration>,
    receiver: Option<Receiver<StreamMessage>>,
    stdin: Option<JoinHandle<Result<(), String>>>,
    stdout: Option<JoinHandle<Result<(), String>>>,
    stderr: Option<JoinHandle<Result<(), String>>>,
    cleanup_required: bool,
    forced_termination: Duration,
}

impl RunningProcess {
    pub(super) fn new(child: Child, forced_termination: Duration) -> Self {
        Self {
            control: Arc::new(ProcessControl::new(&child)),
            child,
            registration: None,
            receiver: None,
            stdin: None,
            stdout: None,
            stderr: None,
            cleanup_required: true,
            forced_termination,
        }
    }

    pub(super) fn register(
        &mut self,
        active: Arc<Mutex<BTreeMap<InvocationId, Arc<ProcessControl>>>>,
        invocation: InvocationId,
    ) -> Result<(), AdapterError> {
        self.registration = Some(
            ActiveRegistration::insert(active, invocation, self.control.clone())
                .map_err(AdapterError::external_failure)?,
        );
        Ok(())
    }

    pub(super) fn start_io(
        &mut self,
        stdin_bytes: Option<Vec<u8>>,
        stdout_maximum: u64,
        stderr_maximum: u64,
    ) -> Result<(), AdapterError> {
        let stdout = self.child.stdout.take().ok_or_else(|| {
            AdapterError::external_failure("spawned process has no owned stdout pipe")
        })?;
        let stderr = self.child.stderr.take().ok_or_else(|| {
            AdapterError::external_failure("spawned process has no owned stderr pipe")
        })?;
        let (sender, receiver) = sync_channel(STREAM_CHANNEL_MESSAGES);
        self.receiver = Some(receiver);
        self.stdin = spawn_stdin_writer(self.child.stdin.take(), stdin_bytes)
            .map_err(AdapterError::external_failure)?;
        self.stdout = Some(
            spawn_reader(Stream::Stdout, stdout, stdout_maximum, sender.clone())
                .map_err(AdapterError::external_failure)?,
        );
        self.stderr = Some(
            spawn_reader(Stream::Stderr, stderr, stderr_maximum, sender)
                .map_err(AdapterError::external_failure)?,
        );
        Ok(())
    }

    pub(super) fn monitor(
        &mut self,
        reports: &mut TerminalReportContext<'_>,
        profile: &ProcessProfile,
        started: Instant,
    ) -> Result<ProcessObservation, AdapterError> {
        let receiver = self.receiver.take().ok_or_else(|| {
            AdapterError::external_failure("process stream receiver is unavailable")
        })?;
        monitor_process(
            &mut self.child,
            &self.control,
            receiver,
            reports,
            profile,
            started,
        )
    }

    pub(super) fn finish_io(&mut self, observed: &ProcessObservation) -> Result<(), String> {
        // A normal monitor return can still describe unresolved termination. In that
        // case request force before joining, without upgrading its recorded outcome.
        if observed.status.is_some() && observed.termination_confirmed {
            self.cleanup_required = false;
        }
        self.cleanup()
    }

    fn cleanup(&mut self) -> Result<(), String> {
        // Readers may be blocked sending to a full channel when reporting stops.
        // Disconnect it before joining, and stop the child before waiting on pipes.
        drop(self.receiver.take());
        if self.cleanup_required {
            terminate_child_immediately(&mut self.child, &self.control);
            let _ = wait_for_owned_descendants_absence(&self.control, self.forced_termination);
            self.cleanup_required = false;
        }
        let stdin = join_io(self.stdin.take(), "stdin writer");
        let stdout = join_io(self.stdout.take(), "stdout reader");
        let stderr = join_io(self.stderr.take(), "stderr reader");
        stdin.and(stdout).and(stderr)
    }
}

impl Drop for RunningProcess {
    fn drop(&mut self) {
        // Also covers partial worker startup and unwinding into the host's panic
        // boundary. Registration drops only after cleanup; the host permit outlives us.
        let _ = self.cleanup();
    }
}

#[cfg(test)]
mod tests;
