use std::{collections::BTreeMap, time::Instant};

use milkdrift_capability::{
    ErrorClass, InvocationEvent, InvocationEventKind, InvocationFailure, InvocationRequest,
    InvocationTerminal, SideEffectClass, TerminalStatus, UsageObservation,
};
use milkdrift_capability_host::{AdapterError, AdapterReporter};

/// Provider-neutral failure facts after adapter-specific status/error mapping.
pub(super) struct ProviderFailure<'a> {
    pub(super) class: ErrorClass,
    pub(super) retryable: bool,
    pub(super) code: &'a str,
    pub(super) message: &'a str,
}

pub(super) fn report_failure(
    request: &InvocationRequest,
    reporter: &dyn AdapterReporter,
    sequence: u64,
    failure: ProviderFailure<'_>,
    started: Instant,
) -> Result<(), AdapterError> {
    report_terminal(
        request,
        reporter,
        sequence,
        TerminalStatus::Failure,
        failure,
        started,
    )
}

pub(super) fn report_uncertain(
    request: &InvocationRequest,
    reporter: &dyn AdapterReporter,
    sequence: u64,
    failure: ProviderFailure<'_>,
    started: Instant,
) -> Result<(), AdapterError> {
    report_terminal(
        request,
        reporter,
        sequence,
        TerminalStatus::Uncertain,
        failure,
        started,
    )
}

fn report_terminal(
    request: &InvocationRequest,
    reporter: &dyn AdapterReporter,
    sequence: u64,
    status: TerminalStatus,
    failure: ProviderFailure<'_>,
    started: Instant,
) -> Result<(), AdapterError> {
    let failure = InvocationFailure::new(
        failure.class,
        failure.retryable,
        failure.code,
        failure.message,
        None,
    )
    .map_err(|_| AdapterError::external_failure("invalid model failure observation"))?;
    let duration = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    let usage = UsageObservation::new(None, None, Some(duration), None, None, BTreeMap::new())
        .map_err(|_| AdapterError::external_failure("invalid model failure usage"))?;
    let terminal = InvocationTerminal::new(
        status,
        Vec::new(),
        Some(failure),
        Some(usage),
        SideEffectClass::Unknown,
    )
    .map_err(|_| AdapterError::external_failure("invalid model failure terminal"))?;
    reporter.invocation(
        InvocationEvent::new(
            request.invocation().clone(),
            sequence,
            InvocationEventKind::Terminal { terminal },
        )
        .map_err(|_| AdapterError::external_failure("invalid model failure event"))?,
    )
}
