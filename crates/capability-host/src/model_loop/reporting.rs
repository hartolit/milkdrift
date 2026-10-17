use super::{adapter_error, limits, rejected};
use crate::{AdapterError, AdapterExecutionContext, AdapterReporter, InvocationDataAccess};
use milkdrift_capability::{
    AdmissionBound, AdmissionMonetaryBound, ArtifactReference, ErrorClass,
    InvocationAdmissionEnvelope, InvocationCounts, InvocationEvent, InvocationEventKind,
    InvocationFailure, InvocationRequest, InvocationTerminal, NestedWorkUsage, SideEffectClass,
    TerminalStatus, UsageObservation,
};
use std::{
    collections::BTreeMap,
    sync::{
        Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::Instant,
};

pub(super) struct ObservedRound {
    pub(super) terminal: InvocationTerminal,
    pub(super) outputs: BTreeMap<String, ArtifactReference>,
}
pub(super) struct LoopReports<'a> {
    request: &'a InvocationRequest,
    context: &'a AdapterExecutionContext,
    reporter: &'a dyn AdapterReporter,
    data: &'a dyn InvocationDataAccess,
    sequence: AtomicU64,
    started: Instant,
    models: u64,
    processes: u64,
    input: Option<u64>,
    output: Option<u64>,
    cost: Option<u64>,
    currency: Option<String>,
    files: Vec<ArtifactReference>,
}
impl<'a> LoopReports<'a> {
    pub(super) fn new(
        request: &'a InvocationRequest,
        context: &'a AdapterExecutionContext,
        reporter: &'a dyn AdapterReporter,
        data: &'a dyn InvocationDataAccess,
    ) -> Self {
        Self {
            request,
            context,
            reporter,
            data,
            sequence: AtomicU64::new(1),
            started: Instant::now(),
            models: 0,
            processes: 0,
            input: Some(0),
            output: Some(0),
            cost: Some(0),
            currency: None,
            files: Vec::new(),
        }
    }
    fn emit(&self, kind: InvocationEventKind) -> Result<(), AdapterError> {
        let sequence = self.sequence.fetch_add(1, Ordering::SeqCst);
        self.reporter.invocation(
            InvocationEvent::new(self.request.invocation().clone(), sequence, kind)
                .map_err(adapter_error)?,
        )
    }
    fn receipt(
        &self,
        request: &InvocationRequest,
        context: &AdapterExecutionContext,
        bytes: &[u8],
    ) -> Result<(), AdapterError> {
        if let Ok(message) = std::str::from_utf8(bytes) {
            let kind = InvocationEventKind::Progress {
                message: message.to_owned(),
                completed_units: None,
                total_units: None,
            };
            if InvocationEvent::new(self.request.invocation().clone(), 1, kind.clone()).is_ok() {
                return self.emit(kind);
            }
        }
        let reference = self
            .data
            .publish_bytes(
                context,
                request,
                "loop_observation",
                "application/vnd.milkdrift.model-loop-observation.v1+json",
                bytes,
                limits(),
            )
            .map_err(adapter_error)?;
        self.emit(InvocationEventKind::Progress {
            message: serde_json::to_string(
                &serde_json::json!({"model_loop_observation": reference}),
            )
            .map_err(adapter_error)?,
            completed_units: None,
            total_units: None,
        })
    }
    pub(super) fn tool(
        &self,
        call: &milkdrift_model::ToolCall,
        result: &str,
    ) -> Result<(), AdapterError> {
        let bytes = serde_json::to_vec(&serde_json::json!({"call": call, "result": result}))
            .map_err(adapter_error)?;
        self.receipt(self.request, self.context, &bytes)
    }
    pub(super) fn file(
        &mut self,
        call: u32,
        text: &str,
    ) -> Result<ArtifactReference, AdapterError> {
        let name = format!("file_{call}");
        let reference = self
            .data
            .publish_bytes(
                self.context,
                self.request,
                &name,
                "text/plain",
                text.as_bytes(),
                limits(),
            )
            .map_err(adapter_error)?;
        self.emit(InvocationEventKind::Output {
            name,
            reference: reference.clone(),
        })?;
        self.files.push(reference.clone());
        Ok(reference)
    }
    pub(super) fn observe(
        &mut self,
        terminal: &InvocationTerminal,
        model: bool,
    ) -> Result<(), AdapterError> {
        if model {
            self.models = self
                .models
                .checked_add(1)
                .ok_or_else(|| rejected("model usage count overflow"))?;
            let usage = terminal.usage();
            self.input = add_observed(self.input, usage.and_then(UsageObservation::input_units))?;
            self.output =
                add_observed(self.output, usage.and_then(UsageObservation::output_units))?;
            self.cost = add_observed(self.cost, usage.and_then(UsageObservation::cost_micros))?;
            if let Some(currency) = usage.and_then(UsageObservation::currency) {
                if self
                    .currency
                    .as_deref()
                    .is_some_and(|prior| prior != currency)
                {
                    return Err(AdapterError::external_failure(
                        "model loop returned inconsistent usage currencies",
                    ));
                }
                self.currency = Some(currency.to_owned());
            }
        } else {
            self.processes = self
                .processes
                .checked_add(1)
                .ok_or_else(|| rejected("process usage count overflow"))?;
        }
        Ok(())
    }
    fn usage(&self) -> Result<UsageObservation, AdapterError> {
        let duration = u64::try_from(self.started.elapsed().as_millis()).map_err(adapter_error)?;
        let currency = self.cost.and(self.currency.clone());
        UsageObservation::new(
            self.input,
            self.output,
            Some(duration),
            if currency.is_some() { self.cost } else { None },
            currency,
            BTreeMap::new(),
        )
        .map(|usage| {
            usage.with_nested_work(NestedWorkUsage::new(
                InvocationCounts::new(self.processes, self.models.saturating_sub(1)),
                0,
            ))
        })
        .map_err(adapter_error)
    }
    pub(super) fn finish(&self, observed: ObservedRound) -> Result<(), AdapterError> {
        for (name, reference) in &observed.outputs {
            self.emit(InvocationEventKind::Output {
                name: name.clone(),
                reference: reference.clone(),
            })?;
        }
        let mut outputs = observed.terminal.outputs().to_vec();
        if matches!(
            observed.terminal.status(),
            TerminalStatus::Success | TerminalStatus::Uncertain
        ) {
            outputs.extend(self.files.iter().cloned());
        }
        let terminal = InvocationTerminal::new(
            observed.terminal.status(),
            outputs,
            observed.terminal.failure().cloned(),
            Some(self.usage()?),
            SideEffectClass::Unknown,
        )
        .map_err(adapter_error)?;
        self.emit(InvocationEventKind::Terminal { terminal })
    }
    pub(super) fn failure(&self, message: &str) -> Result<(), AdapterError> {
        let terminal = InvocationTerminal::new(
            TerminalStatus::Failure,
            Vec::new(),
            Some(
                InvocationFailure::new(
                    ErrorClass::InvalidRequest,
                    false,
                    "model_tool_loop_refused",
                    message,
                    None,
                )
                .map_err(adapter_error)?,
            ),
            Some(self.usage()?),
            SideEffectClass::Unknown,
        )
        .map_err(adapter_error)?;
        self.emit(InvocationEventKind::Terminal { terminal })
    }
    pub(super) fn cancelled(&self) -> Result<(), AdapterError> {
        let terminal = InvocationTerminal::new(
            TerminalStatus::Cancelled,
            Vec::new(),
            None,
            Some(self.usage()?),
            SideEffectClass::Unknown,
        )
        .map_err(adapter_error)?;
        self.emit(InvocationEventKind::Terminal { terminal })
    }
}
fn add_observed(left: Option<u64>, right: Option<u64>) -> Result<Option<u64>, AdapterError> {
    left.zip(right)
        .map(|(left, right)| {
            left.checked_add(right)
                .ok_or_else(|| rejected("usage addition overflow"))
        })
        .transpose()
}

struct RoundState {
    next: u64,
    terminal: Option<InvocationTerminal>,
    outputs: BTreeMap<String, ArtifactReference>,
}
pub(super) struct RoundReports<'a, 'b> {
    parent: &'a LoopReports<'b>,
    request: &'a InvocationRequest,
    context: &'a AdapterExecutionContext,
    state: Mutex<RoundState>,
}
impl<'a, 'b> RoundReports<'a, 'b> {
    pub(super) fn new(
        parent: &'a LoopReports<'b>,
        request: &'a InvocationRequest,
        context: &'a AdapterExecutionContext,
    ) -> Self {
        Self {
            parent,
            request,
            context,
            state: Mutex::new(RoundState {
                next: 1,
                terminal: None,
                outputs: BTreeMap::new(),
            }),
        }
    }
    pub(super) fn finish(self) -> Result<ObservedRound, AdapterError> {
        let state = self
            .state
            .into_inner()
            .map_err(|_| rejected("tool report owner unavailable"))?;
        Ok(ObservedRound {
            terminal: state.terminal.ok_or_else(|| {
                AdapterError::external_failure("tool returned without durable terminal evidence")
            })?,
            outputs: state.outputs,
        })
    }
}
impl AdapterReporter for RoundReports<'_, '_> {
    fn invocation(&self, event: InvocationEvent) -> Result<(), AdapterError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| rejected("tool report owner unavailable"))?;
        if event.invocation() != self.request.invocation()
            || event.sequence() != state.next
            || state.terminal.is_some()
        {
            return Err(AdapterError::external_failure(
                "tool report sequence or identity mismatch",
            ));
        }
        let bytes = serde_json::to_vec(&event).map_err(adapter_error)?;
        self.parent.receipt(self.request, self.context, &bytes)?;
        match event.kind() {
            InvocationEventKind::Output { name, reference } => {
                if state
                    .outputs
                    .insert(name.clone(), reference.clone())
                    .is_some()
                {
                    return Err(AdapterError::external_failure(
                        "tool duplicated an output name",
                    ));
                }
            }
            InvocationEventKind::Terminal { terminal } => state.terminal = Some(terminal.clone()),
            InvocationEventKind::Progress { .. } => {}
        }
        state.next = state
            .next
            .checked_add(1)
            .ok_or_else(|| rejected("tool sequence overflow"))?;
        Ok(())
    }
    fn heartbeat(&self) -> Result<(), AdapterError> {
        self.parent.reporter.heartbeat()
    }
}

pub(super) fn fits(
    actual: &InvocationAdmissionEnvelope,
    maximum: &InvocationAdmissionEnvelope,
) -> Result<(), AdapterError> {
    let within =
        |actual: &AdmissionBound<u64>, maximum: &AdmissionBound<u64>| match (actual, maximum) {
            (AdmissionBound::NotApplicable, _) => true,
            (AdmissionBound::Bounded(a), AdmissionBound::Bounded(m)) => a <= m,
            _ => false,
        };
    let cost = match (actual.monetary_cost(), maximum.monetary_cost()) {
        (AdmissionBound::NotApplicable, _) => true,
        (AdmissionBound::Bounded(a), AdmissionBound::Bounded(m)) => {
            a.currency() == m.currency() && a.maximum_micros() <= m.maximum_micros()
        }
        _ => false,
    };
    if !within(actual.input_units(), maximum.input_units())
        || !within(actual.output_units(), maximum.output_units())
        || !within(actual.artifact_bytes(), maximum.artifact_bytes())
        || !within(
            actual.execution_duration_ms(),
            maximum.execution_duration_ms(),
        )
        || !cost
        || actual.nested_invocations().is_some()
    {
        return Err(rejected(
            "nested entry exceeds the complete model-loop reservation",
        ));
    }
    Ok(())
}
fn amount(bound: &AdmissionBound<u64>) -> Result<u64, AdapterError> {
    match bound {
        AdmissionBound::Bounded(value) => Ok(*value),
        AdmissionBound::NotApplicable => Ok(0),
        AdmissionBound::Unknown => {
            Err(rejected("model-loop admission requires enforceable bounds"))
        }
    }
}
fn multiplied(value: u64, count: u64) -> Result<u64, AdapterError> {
    value
        .checked_mul(count)
        .ok_or_else(|| rejected("model-loop reservation overflow"))
}
pub(super) fn reserve(
    model: &InvocationAdmissionEnvelope,
    processes: &[&InvocationAdmissionEnvelope],
    maximum_calls: u32,
) -> Result<InvocationAdmissionEnvelope, AdapterError> {
    let calls = u64::from(maximum_calls);
    let models = calls
        .checked_add(1)
        .ok_or_else(|| rejected("model round overflow"))?;
    let mut process_duration = 0_u64;
    let mut process_bytes = 0_u64;
    for process in processes {
        if amount(process.input_units())? != 0
            || amount(process.output_units())? != 0
            || !matches!(process.monetary_cost(), AdmissionBound::NotApplicable)
            || process.nested_invocations().is_some()
        {
            return Err(rejected(
                "process tools require ordinary unbilled process envelopes",
            ));
        }
        process_duration = process_duration.max(amount(process.execution_duration_ms())?);
        process_bytes = process_bytes.max(amount(process.artifact_bytes())?);
    }
    let cost = match model.monetary_cost() {
        AdmissionBound::Bounded(cost) => AdmissionBound::Bounded(
            AdmissionMonetaryBound::new(
                multiplied(cost.maximum_micros(), models)?,
                cost.currency(),
            )
            .map_err(adapter_error)?,
        ),
        AdmissionBound::NotApplicable => AdmissionBound::NotApplicable,
        AdmissionBound::Unknown => return Err(rejected("model-loop monetary bound is unknown")),
    };
    let duration = multiplied(amount(model.execution_duration_ms())?, models)?
        .checked_add(multiplied(process_duration, calls)?)
        .ok_or_else(|| rejected("model-loop duration overflow"))?;
    let bytes = multiplied(amount(model.artifact_bytes())?, models)?
        .checked_add(multiplied(process_bytes, calls)?)
        .and_then(|bytes| bytes.checked_add(2_097_152))
        .ok_or_else(|| rejected("model-loop artifact bound overflow"))?;
    Ok(InvocationAdmissionEnvelope::new(
        model.unit(),
        AdmissionBound::Bounded(multiplied(amount(model.input_units())?, models)?),
        AdmissionBound::Bounded(multiplied(amount(model.output_units())?, models)?),
        AdmissionBound::Bounded(bytes),
        cost,
    )
    .with_execution_duration_ms(AdmissionBound::Bounded(duration))
    .with_nested_invocations(InvocationCounts::new(
        if processes.is_empty() { 0 } else { calls },
        calls,
    )))
}
