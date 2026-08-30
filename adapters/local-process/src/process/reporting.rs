use std::{collections::BTreeMap, process::ExitStatus, time::Instant};

use milkdrift_capability::{
    ErrorClass, InvocationEvent, InvocationEventKind, InvocationFailure, InvocationId,
    InvocationTerminal, SideEffectClass, TerminalStatus, UsageObservation,
};
use milkdrift_capability_host::{AdapterError, AdapterReporter};

use super::{bounded, monitor::Termination};

pub(super) fn report(
    reporter: &dyn AdapterReporter,
    invocation: &InvocationId,
    sequence: &mut u64,
    kind: InvocationEventKind,
) -> Result<(), AdapterError> {
    let event = InvocationEvent::new(invocation.clone(), *sequence, kind)
        .map_err(|error| AdapterError::external_failure(error.to_string()))?;
    reporter.invocation(event)?;
    *sequence = sequence
        .checked_add(1)
        .ok_or_else(|| AdapterError::external_failure("invocation sequence overflow"))?;
    Ok(())
}

pub(super) fn report_rejected(
    reporter: &dyn AdapterReporter,
    invocation: &InvocationId,
    sequence: &mut u64,
    class: ErrorClass,
    code: &str,
    message: &str,
) -> Result<(), AdapterError> {
    let failure = InvocationFailure::new(class, false, code, bounded(message), None)
        .map_err(|error| AdapterError::rejected(error.to_string()))?;
    let terminal = InvocationTerminal::new(
        TerminalStatus::Rejected,
        Vec::new(),
        Some(failure),
        None,
        SideEffectClass::None,
    )
    .map_err(|error| AdapterError::rejected(error.to_string()))?;
    report(
        reporter,
        invocation,
        sequence,
        InvocationEventKind::Terminal { terminal },
    )
}

/// Shared event sink and immutable terminal facts for one entered process invocation.
pub(super) struct TerminalReportContext<'a> {
    reporter: &'a dyn AdapterReporter,
    invocation: &'a InvocationId,
    sequence: &'a mut u64,
    side_effect: SideEffectClass,
    started: Instant,
}

impl<'a> TerminalReportContext<'a> {
    pub(super) fn new(
        reporter: &'a dyn AdapterReporter,
        invocation: &'a InvocationId,
        sequence: &'a mut u64,
        side_effect: SideEffectClass,
        started: Instant,
    ) -> Self {
        Self {
            reporter,
            invocation,
            sequence,
            side_effect,
            started,
        }
    }

    pub(super) fn report(&mut self, kind: InvocationEventKind) -> Result<(), AdapterError> {
        report(self.reporter, self.invocation, self.sequence, kind)
    }

    pub(super) fn heartbeat(&self) -> Result<(), AdapterError> {
        self.reporter.heartbeat()
    }

    pub(super) fn failure(
        &mut self,
        class: ErrorClass,
        code: &str,
        message: &str,
    ) -> Result<(), AdapterError> {
        let failure = InvocationFailure::new(class, false, code, bounded(message), None)
            .map_err(|error| AdapterError::external_failure(error.to_string()))?;
        let terminal = InvocationTerminal::new(
            TerminalStatus::Failure,
            Vec::new(),
            Some(failure),
            usage(self.started),
            self.side_effect,
        )
        .map_err(|error| AdapterError::external_failure(error.to_string()))?;
        self.report(InvocationEventKind::Terminal { terminal })
    }

    pub(super) fn uncertain(&mut self, code: &str, message: &str) -> Result<(), AdapterError> {
        let failure =
            InvocationFailure::new(ErrorClass::Unknown, false, code, bounded(message), None)
                .map_err(|error| AdapterError::external_failure(error.to_string()))?;
        let terminal = InvocationTerminal::new(
            TerminalStatus::Uncertain,
            Vec::new(),
            Some(failure),
            usage(self.started),
            self.side_effect,
        )
        .map_err(|error| AdapterError::external_failure(error.to_string()))?;
        self.report(InvocationEventKind::Terminal { terminal })
    }

    pub(super) fn for_termination(
        &mut self,
        termination: Termination,
        group_absent: bool,
    ) -> Result<(), AdapterError> {
        match termination {
            Termination::Cancelled if group_absent => {
                let terminal = InvocationTerminal::new(
                    TerminalStatus::Cancelled,
                    Vec::new(),
                    None,
                    usage(self.started),
                    self.side_effect,
                )
                .map_err(|error| AdapterError::external_failure(error.to_string()))?;
                self.report(InvocationEventKind::Terminal { terminal })
            }
            Termination::Cancelled => self.uncertain(
                "process_descendants_unresolved",
                "cancellation was requested but owned process-group disappearance was not proven",
            ),
            Termination::TimedOut if group_absent => self.failure(
                ErrorClass::Provider,
                "process_timeout",
                "process exceeded its wall timeout and the owned group was terminated",
            ),
            Termination::OutputOverflow if group_absent => self.failure(
                ErrorClass::Adapter,
                "process_output_overflow",
                "process output exceeded a terminate-on-overflow bound",
            ),
            Termination::UnexpectedDescendants if group_absent => self.failure(
                ErrorClass::Adapter,
                "process_descendant_contract_violated",
                "the immediate process exited while owned descendants remained; the group was terminated",
            ),
            Termination::TimedOut
            | Termination::OutputOverflow
            | Termination::UnexpectedDescendants
            | Termination::Unresolved => self.uncertain(
                "process_termination_unresolved",
                "process termination or descendant cleanup could not be proven",
            ),
        }
    }
}

pub(super) fn usage(started: Instant) -> Option<UsageObservation> {
    let duration = u64::try_from(started.elapsed().as_millis()).ok();
    UsageObservation::new(None, None, duration, None, None, BTreeMap::new()).ok()
}

pub(super) fn exit_failure(status: &ExitStatus) -> (String, String) {
    if let Some(code) = status.code() {
        return (
            "process_nonzero_exit".to_owned(),
            format!("process exited with status code {code}"),
        );
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        if let Some(signal) = status.signal() {
            return (
                "process_signal_exit".to_owned(),
                format!("process terminated by signal {signal}"),
            );
        }
    }
    (
        "process_unknown_exit".to_owned(),
        "process exited without a portable status code".to_owned(),
    )
}
