use std::{
    process::{Child, ExitStatus},
    sync::{
        atomic::Ordering,
        mpsc::{Receiver, RecvTimeoutError},
    },
    time::{Duration, Instant},
};

use milkdrift_capability::InvocationEventKind;
use milkdrift_capability_host::AdapterError;

use crate::config::{OverflowAction, ProcessProfile};

use super::{
    platform::{ProcessControl, wait_for_owned_descendants_absence},
    reporting::TerminalReportContext,
    streams::{Stream, StreamMessage, progress_message},
};

const POLL_INTERVAL: Duration = Duration::from_millis(10);

pub(super) struct ProcessObservation {
    pub(super) status: Option<ExitStatus>,
    pub(super) termination: Option<Termination>,
    pub(super) stdout: Vec<u8>,
    pub(super) stderr: Vec<u8>,
    pub(super) stdout_overflow: bool,
    pub(super) stderr_overflow: bool,
    pub(super) termination_confirmed: bool,
}

#[derive(Clone, Copy)]
pub(super) enum Termination {
    Cancelled,
    TimedOut,
    OutputOverflow,
    UnexpectedDescendants,
    Unresolved,
}

pub(super) fn monitor_process(
    child: &mut Child,
    control: &ProcessControl,
    receiver: Receiver<StreamMessage>,
    reports: &mut TerminalReportContext<'_>,
    profile: &ProcessProfile,
    started: Instant,
) -> Result<ProcessObservation, AdapterError> {
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let mut stdout_overflow = false;
    let mut stderr_overflow = false;
    let mut stdout_closed = false;
    let mut stderr_closed = false;
    let mut stdout_progress = 0_u16;
    let mut stderr_progress = 0_u16;
    let mut termination = None;
    let mut graceful_at = None;
    let mut forced_at = None;
    let mut next_heartbeat = started + Duration::from_millis(profile.limits.heartbeat_interval_ms);
    let wall_deadline = started + Duration::from_millis(profile.limits.wall_timeout_ms);
    let mut status = None;
    loop {
        match receiver.recv_timeout(POLL_INTERVAL) {
            Ok(StreamMessage::Data(stream, bytes)) => {
                let (capture, policy, count) = match stream {
                    Stream::Stdout => (&mut stdout, &profile.stdout, &mut stdout_progress),
                    Stream::Stderr => (&mut stderr, &profile.stderr, &mut stderr_progress),
                };
                capture.extend_from_slice(&bytes);
                if policy.stream_progress && *count < policy.max_progress_events {
                    let message = progress_message(stream, &bytes);
                    reports.report(InvocationEventKind::Progress {
                        message,
                        completed_units: None,
                        total_units: None,
                    })?;
                    *count = count.saturating_add(1);
                }
            }
            Ok(StreamMessage::Overflow(stream)) => {
                let terminate = match stream {
                    Stream::Stdout => {
                        stdout_overflow = true;
                        profile.stdout.overflow_action == OverflowAction::Terminate
                    }
                    Stream::Stderr => {
                        stderr_overflow = true;
                        profile.stderr.overflow_action == OverflowAction::Terminate
                    }
                };
                if terminate && termination.is_none() {
                    termination = Some(Termination::OutputOverflow);
                }
            }
            Ok(StreamMessage::Closed(Stream::Stdout)) => stdout_closed = true,
            Ok(StreamMessage::Closed(Stream::Stderr)) => stderr_closed = true,
            Ok(StreamMessage::Failed(stream, kind)) => {
                let stream_name = match stream {
                    Stream::Stdout => "stdout",
                    Stream::Stderr => "stderr",
                };
                return Err(AdapterError::external_failure(format!(
                    "{stream_name} reader failed: {kind:?}"
                )));
            }
            Err(RecvTimeoutError::Disconnected) => {
                stdout_closed = true;
                stderr_closed = true;
            }
            Err(RecvTimeoutError::Timeout) => {}
        }
        if status.is_none() {
            status = child.try_wait().map_err(|error| {
                AdapterError::external_failure(format!("process wait failed: {:?}", error.kind()))
            })?;
        }
        let now = Instant::now();
        if control.cancel_requested.load(Ordering::SeqCst) && termination.is_none() {
            termination = Some(Termination::Cancelled);
        }
        let owned_descendants_absent = status.is_some() && control.owned_descendants_absent();
        if status.is_some() && !owned_descendants_absent && termination.is_none() {
            termination = Some(Termination::UnexpectedDescendants);
        }
        if now >= wall_deadline && termination.is_none() {
            termination = Some(Termination::TimedOut);
        }
        if termination.is_some() && !owned_descendants_absent && graceful_at.is_none() {
            control
                .request_graceful()
                .map_err(AdapterError::external_failure)?;
            #[cfg(not(unix))]
            if status.is_none() {
                child
                    .kill()
                    .map_err(|error| AdapterError::external_failure(error.to_string()))?;
            }
            graceful_at = Some(now);
        }
        if !owned_descendants_absent
            && graceful_at.is_some_and(|at| {
                now.duration_since(at)
                    >= Duration::from_millis(profile.limits.graceful_termination_ms)
            })
            && forced_at.is_none()
        {
            control
                .request_force()
                .map_err(AdapterError::external_failure)?;
            if status.is_none() {
                child
                    .kill()
                    .map_err(|error| AdapterError::external_failure(error.to_string()))?;
            }
            forced_at = Some(now);
        }
        if !owned_descendants_absent
            && forced_at.is_some_and(|at| {
                now.duration_since(at)
                    >= Duration::from_millis(profile.limits.forced_termination_ms)
            })
        {
            termination = Some(Termination::Unresolved);
            break;
        }
        if now >= next_heartbeat {
            reports.heartbeat()?;
            next_heartbeat = now + Duration::from_millis(profile.limits.heartbeat_interval_ms);
        }
        if status.is_some() && owned_descendants_absent && stdout_closed && stderr_closed {
            break;
        }
    }
    if status.is_none() {
        status = child.try_wait().map_err(|error| {
            AdapterError::external_failure(format!("final process wait failed: {:?}", error.kind()))
        })?;
    }
    let owned_descendants_absent = if termination.is_some() {
        wait_for_owned_descendants_absence(
            control,
            Duration::from_millis(profile.limits.forced_termination_ms),
        )
    } else {
        control.owned_descendants_absent()
    };
    Ok(ProcessObservation {
        status,
        termination,
        stdout,
        stderr,
        stdout_overflow,
        stderr_overflow,
        termination_confirmed: status.is_some() && owned_descendants_absent,
    })
}
