//! Retain the child, cancellation registration, and every started I/O worker until cleanup.

use std::{
    collections::BTreeMap,
    process::Child,
    sync::{
        Arc, Mutex,
        mpsc::{Receiver, sync_channel},
    },
    time::{Duration, Instant},
};

use milkdrift_capability::InvocationId;
use milkdrift_capability_host::AdapterError;

use super::{
    monitor::{MonitorIo, ProcessObservation, monitor_process},
    platform::{ActiveRegistration, ProcessControl, terminate_child_until},
    reporting::TerminalReportContext,
    streams::{
        IoCancellation, IoCompletion, IoWorker, Stream, StreamMessage, join_io, pipe_reader,
        pipe_writer, spawn_reader, spawn_stdin_writer,
    },
};
use crate::config::ProcessProfile;

const STREAM_CHANNEL_MESSAGES: usize = 16;

pub(super) struct IoCleanup {
    pub(super) stdin: Result<IoCompletion, String>,
    pub(super) stdout: Result<IoCompletion, String>,
    pub(super) stderr: Result<IoCompletion, String>,
}

impl IoCleanup {
    pub(super) fn error(&self) -> Option<&str> {
        [&self.stdin, &self.stdout, &self.stderr]
            .into_iter()
            .find_map(|result| result.as_ref().err().map(String::as_str))
    }
}

pub(super) struct RunningProcess {
    child: Child,
    control: Arc<ProcessControl>,
    registration: Option<ActiveRegistration>,
    receiver: Option<Receiver<StreamMessage>>,
    stdin: Option<IoWorker>,
    stdout: Option<IoWorker>,
    stderr: Option<IoWorker>,
    io_cancel: IoCancellation,
    cleanup_deadline: Option<Instant>,
    cleanup_required: bool,
    forced_termination: Duration,
    #[cfg(test)]
    fail_reader_spawn: Option<Stream>,
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
            io_cancel: IoCancellation::default(),
            cleanup_deadline: None,
            cleanup_required: true,
            forced_termination,
            #[cfg(test)]
            fail_reader_spawn: None,
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
        self.stdin = spawn_stdin_writer(
            self.child.stdin.take().map(pipe_writer),
            stdin_bytes,
            self.io_cancel.input.clone(),
        )
        .map_err(AdapterError::external_failure)?;
        #[cfg(test)]
        if self.fail_reader_spawn == Some(Stream::Stdout) {
            return Err(AdapterError::external_failure(
                "injected stdout worker spawn failure",
            ));
        }
        self.stdout = Some(
            spawn_reader(
                Stream::Stdout,
                pipe_reader(stdout),
                stdout_maximum,
                sender.clone(),
                self.io_cancel.output.clone(),
            )
            .map_err(AdapterError::external_failure)?,
        );
        #[cfg(test)]
        if self.fail_reader_spawn == Some(Stream::Stderr) {
            return Err(AdapterError::external_failure(
                "injected stderr worker spawn failure",
            ));
        }
        self.stderr = Some(
            spawn_reader(
                Stream::Stderr,
                pipe_reader(stderr),
                stderr_maximum,
                sender,
                self.io_cancel.output.clone(),
            )
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
        let receiver = self.receiver.as_ref().ok_or_else(|| {
            AdapterError::external_failure("process stream receiver is unavailable")
        })?;
        monitor_process(
            &mut self.child,
            &self.control,
            MonitorIo {
                receiver,
                cancellation: &self.io_cancel,
                deadline: &mut self.cleanup_deadline,
            },
            reports,
            profile,
            started,
        )
    }

    pub(super) fn finish_io(&mut self, observed: &mut ProcessObservation) -> IoCleanup {
        // A normal monitor return can still describe unresolved termination. In that
        // case request force before joining, without upgrading its recorded outcome.
        if observed.status.is_some() && observed.termination_confirmed {
            self.cleanup_required = false;
        }
        // Retain the bounded queue while workers stop. Reporting may have consumed
        // the final drain allowance while EOF/data was already waiting in it.
        let receiver = self.receiver.take();
        let joined = self.cleanup();
        if let Some(receiver) = receiver {
            for message in receiver.try_iter() {
                match message {
                    StreamMessage::Data(Stream::Stdout, bytes) => observed.stdout.extend(bytes),
                    StreamMessage::Data(Stream::Stderr, bytes) => observed.stderr.extend(bytes),
                    StreamMessage::Overflow(Stream::Stdout) => observed.stdout_overflow = true,
                    StreamMessage::Overflow(Stream::Stderr) => observed.stderr_overflow = true,
                    StreamMessage::Closed(Stream::Stdout) => observed.stdout_closed = true,
                    StreamMessage::Closed(Stream::Stderr) => observed.stderr_closed = true,
                    StreamMessage::Failed(..) => {} // The joined worker retains the error.
                }
            }
        }
        observed.stdout_closed |= joined.stdout == Ok(IoCompletion::Complete);
        observed.stderr_closed |= joined.stderr == Ok(IoCompletion::Complete);
        joined
    }

    fn cleanup(&mut self) -> IoCleanup {
        // Readers may be blocked sending to a full channel when reporting stops.
        // Disconnect it before joining, and stop the child before waiting on pipes.
        drop(self.receiver.take());
        self.io_cancel
            .input
            .store(true, std::sync::atomic::Ordering::Release);
        self.io_cancel
            .output
            .store(true, std::sync::atomic::Ordering::Release);
        if self.cleanup_required {
            let deadline = *self
                .cleanup_deadline
                .get_or_insert_with(|| Instant::now() + self.forced_termination);
            terminate_child_until(&mut self.child, &self.control, deadline);
            self.cleanup_required = false;
        }
        let stdin = join_io(self.stdin.take(), "stdin writer");
        let stdout = join_io(self.stdout.take(), "stdout reader");
        let stderr = join_io(self.stderr.take(), "stderr reader");
        IoCleanup {
            stdin,
            stdout,
            stderr,
        }
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
