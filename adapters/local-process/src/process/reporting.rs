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

#[allow(clippy::too_many_arguments)]
pub(super) fn terminal_failure(
    reporter: &dyn AdapterReporter,
    invocation: &InvocationId,
    sequence: &mut u64,
    class: ErrorClass,
    code: &str,
    message: &str,
    side_effect: SideEffectClass,
    started: Instant,
) -> Result<(), AdapterError> {
    let failure = InvocationFailure::new(class, false, code, bounded(message), None)
        .map_err(|error| AdapterError::external_failure(error.to_string()))?;
    let terminal = InvocationTerminal::new(
        TerminalStatus::Failure,
        Vec::new(),
        Some(failure),
        usage(started),
        side_effect,
    )
    .map_err(|error| AdapterError::external_failure(error.to_string()))?;
    report(
        reporter,
        invocation,
        sequence,
        InvocationEventKind::Terminal { terminal },
    )
}

#[allow(clippy::too_many_arguments)]
pub(super) fn terminal_uncertain(
    reporter: &dyn AdapterReporter,
    invocation: &InvocationId,
    sequence: &mut u64,
    code: &str,
    message: &str,
    side_effect: SideEffectClass,
    started: Instant,
) -> Result<(), AdapterError> {
    let failure = InvocationFailure::new(ErrorClass::Unknown, false, code, bounded(message), None)
        .map_err(|error| AdapterError::external_failure(error.to_string()))?;
    let terminal = InvocationTerminal::new(
        TerminalStatus::Uncertain,
        Vec::new(),
        Some(failure),
        usage(started),
        side_effect,
    )
    .map_err(|error| AdapterError::external_failure(error.to_string()))?;
    report(
        reporter,
        invocation,
        sequence,
        InvocationEventKind::Terminal { terminal },
    )
}

#[allow(clippy::too_many_arguments)]
pub(super) fn terminal_for_termination(
    reporter: &dyn AdapterReporter,
    invocation: &InvocationId,
    sequence: &mut u64,
    termination: Termination,
    side_effect: SideEffectClass,
    started: Instant,
    group_absent: bool,
) -> Result<(), AdapterError> {
    match termination {
        Termination::Cancelled if group_absent => {
            let terminal = InvocationTerminal::new(
                TerminalStatus::Cancelled,
                Vec::new(),
                None,
                usage(started),
                side_effect,
            )
            .map_err(|error| AdapterError::external_failure(error.to_string()))?;
            report(
                reporter,
                invocation,
                sequence,
                InvocationEventKind::Terminal { terminal },
            )
        }
        Termination::Cancelled => terminal_uncertain(
            reporter,
            invocation,
            sequence,
            "process_descendants_unresolved",
            "cancellation was requested but owned process-group disappearance was not proven",
            side_effect,
            started,
        ),
        Termination::TimedOut if group_absent => terminal_failure(
            reporter,
            invocation,
            sequence,
            ErrorClass::Provider,
            "process_timeout",
            "process exceeded its wall timeout and the owned group was terminated",
            side_effect,
            started,
        ),
        Termination::OutputOverflow if group_absent => terminal_failure(
            reporter,
            invocation,
            sequence,
            ErrorClass::Adapter,
            "process_output_overflow",
            "process output exceeded a terminate-on-overflow bound",
            side_effect,
            started,
        ),
        Termination::UnexpectedDescendants if group_absent => terminal_failure(
            reporter,
            invocation,
            sequence,
            ErrorClass::Adapter,
            "process_descendant_contract_violated",
            "the immediate process exited while owned descendants remained; the group was terminated",
            side_effect,
            started,
        ),
        Termination::TimedOut
        | Termination::OutputOverflow
        | Termination::UnexpectedDescendants
        | Termination::Unresolved => terminal_uncertain(
            reporter,
            invocation,
            sequence,
            "process_termination_unresolved",
            "process termination or descendant cleanup could not be proven",
            side_effect,
            started,
        ),
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
