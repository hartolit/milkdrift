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

use super::{
    MaterializationLedger, ModelEndpointAdapter, PROVIDER_METADATA_MEDIA, PreparedOutput,
    RESPONSE_MEDIA, STRUCTURED_MEDIA, TOOL_CALLS_MEDIA,
};
use milkdrift_capability::{BoundedJson, InvocationAdmissionEnvelope};
use milkdrift_model::{ModelResponse, ModelResponseDocument};
use serde_json::json;

impl ModelEndpointAdapter {
    #[allow(clippy::too_many_arguments)] // Publication binds the completed response to its exact prepared accounting facts.
    pub(super) fn publish_response(
        &self,
        context: &milkdrift_capability_host::AdapterExecutionContext,
        request: &InvocationRequest,
        reporter: &dyn AdapterReporter,
        mut response: ModelResponse,
        mut sequence: u64,
        started: Instant,
        envelope: &InvocationAdmissionEnvelope,
        request_digest: &str,
    ) -> Result<(), AdapterError> {
        let accounting = self.profile.accounted_usage(&response);
        let violation = if response
            .usage()
            .input_units
            .zip(envelope.input_units().bounded())
            .is_some_and(|(used, bound)| used > *bound)
            || response
                .usage()
                .output_units
                .zip(envelope.output_units().bounded())
                .is_some_and(|(used, bound)| used > *bound)
        {
            Some("provider usage exceeds the prepared token limits")
        } else {
            accounting.as_ref().err().copied()
        };
        // A contradictory invoice is raw evidence, never a tariff calculation. Keep it in
        // the response while withholding a monetary settlement for the uncertain terminal.
        let accounted = accounting.unwrap_or_else(|_| {
            let mut usage = response.usage().clone();
            usage.cost_micros = None;
            usage.currency = None;
            usage
        });
        let mut metadata = response.provider_metadata().clone();
        metadata.insert(
            milkdrift_capability::ExtensionKey::new("org.milkdrift/model-accounting")
                .map_err(|_| AdapterError::external_failure("invalid accounting metadata key"))?,
            BoundedJson::new(json!({
                "billing": self.profile.billing(), "token_limits": self.profile.token_limits(),
                "request_digest": request_digest, "reservation_envelope": envelope,
                "accounted_cost_micros": accounted.cost_micros, "currency": accounted.currency,
                "cost_basis": if violation.is_some() { "unresolved" } else { match self.profile.billing() {
                    crate::BillingTerms::Unknown => "provider_reported_or_unknown",
                    crate::BillingTerms::Unbilled { .. } => "operator_unbilled",
                    crate::BillingTerms::TextTariff { .. } => "tariff_calculated",
                } },
                "accounting_error": violation,
            }))
            .map_err(|_| AdapterError::external_failure("accounting metadata exceeds bounds"))?,
        );
        response = ModelResponse::new(
            response.text().to_owned(),
            response.structured().cloned(),
            response.tool_calls().to_vec(),
            response.finish_reason(),
            response.usage().clone(),
            metadata,
        )
        .map_err(|_| AdapterError::external_failure("invalid accounting response"))?;
        let limits = self.materialization_limits();
        let canonical = ModelResponseDocument::new(response.clone())
            .to_canonical_json()
            .map_err(|_| {
                AdapterError::external_failure("canonical model response encoding failed")
            })?;
        let mut outputs = vec![PreparedOutput {
            name: "model_response",
            media_type: RESPONSE_MEDIA,
            bytes: canonical,
        }];
        if !response.text().is_empty() {
            outputs.push(PreparedOutput {
                name: "final_text",
                media_type: "text/plain",
                bytes: response.text().as_bytes().to_vec(),
            });
        }
        if let Some(structured) = response.structured() {
            let bytes = serde_json::to_vec(structured.value())
                .map_err(|_| AdapterError::external_failure("structured output encoding failed"))?;
            outputs.push(PreparedOutput {
                name: "structured_output",
                media_type: STRUCTURED_MEDIA,
                bytes,
            });
        }
        if !response.tool_calls().is_empty() {
            let bytes = serde_json::to_vec(response.tool_calls())
                .map_err(|_| AdapterError::external_failure("tool call encoding failed"))?;
            outputs.push(PreparedOutput {
                name: "tool_calls",
                media_type: TOOL_CALLS_MEDIA,
                bytes,
            });
        }
        if !response.provider_metadata().is_empty() {
            let bytes = serde_json::to_vec(response.provider_metadata())
                .map_err(|_| AdapterError::external_failure("provider metadata encoding failed"))?;
            outputs.push(PreparedOutput {
                name: "provider_metadata",
                media_type: PROVIDER_METADATA_MEDIA,
                bytes,
            });
        }
        let mut output_ledger = MaterializationLedger::new(limits);
        if outputs
            .iter()
            .try_for_each(|output| output_ledger.record(output.bytes.len()))
            .is_err()
        {
            return report_failure(
                request,
                reporter,
                sequence,
                ProviderFailure {
                    class: ErrorClass::Adapter,
                    retryable: false,
                    code: "artifact_output_bounds",
                    message: "aggregate model output artifacts exceed the configured bound",
                },
                started,
            );
        }
        let mut published = Vec::with_capacity(outputs.len());
        for output in outputs {
            let reference = match self.data.publish_bytes(
                context,
                request,
                output.name,
                output.media_type,
                &output.bytes,
                limits,
            ) {
                Ok(value) => value,
                Err(_) => {
                    return report_failure(
                        request,
                        reporter,
                        sequence,
                        ProviderFailure {
                            class: ErrorClass::Adapter,
                            retryable: false,
                            code: "artifact_publication",
                            message: "model output artifact publication failed",
                        },
                        started,
                    );
                }
            };
            published.push((output.name, reference));
        }
        let output_refs = published
            .iter()
            .map(|(_, reference)| reference.clone())
            .collect::<Vec<_>>();
        for (name, reference) in published {
            reporter.invocation(
                InvocationEvent::new(
                    request.invocation().clone(),
                    sequence,
                    InvocationEventKind::Output {
                        name: name.to_owned(),
                        reference,
                    },
                )
                .map_err(|_| AdapterError::external_failure("invalid model output event"))?,
            )?;
            sequence = sequence
                .checked_add(1)
                .ok_or_else(|| AdapterError::external_failure("model report sequence overflow"))?;
        }
        let duration = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        let usage = &accounted;
        let observed = UsageObservation::new(
            usage.input_units,
            usage.output_units,
            Some(duration),
            usage.cost_micros,
            usage.currency.clone(),
            BTreeMap::new(),
        )
        .map_err(|_| AdapterError::external_failure("invalid model usage observation"))?;
        let terminal = InvocationTerminal::new(
            if violation.is_some() {
                TerminalStatus::Uncertain
            } else {
                TerminalStatus::Success
            },
            output_refs,
            violation
                .map(|message| {
                    milkdrift_capability::InvocationFailure::new(
                        ErrorClass::Unknown,
                        false,
                        "model_accounting_conflict",
                        message,
                        None,
                    )
                })
                .transpose()
                .map_err(|_| AdapterError::external_failure("invalid accounting failure"))?,
            Some(observed),
            SideEffectClass::Unknown,
        )
        .map_err(|_| AdapterError::external_failure("invalid model terminal event"))?;
        reporter.invocation(
            InvocationEvent::new(
                request.invocation().clone(),
                sequence,
                InvocationEventKind::Terminal { terminal },
            )
            .map_err(|_| AdapterError::external_failure("invalid model terminal event"))?,
        )
    }
}
