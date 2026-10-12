#![deny(clippy::arithmetic_side_effects)]

use super::WorkflowRecord;
use crate::{ExecutionDispatch, RuntimeError};
use milkdrift_authority::AuthorityBudget;
use milkdrift_capability::{AdmissionBound, InvocationAdmissionEnvelope};

/// Reserve enforceable maxima permanently in the cumulative workflow account. A terminal
/// observation can prove actual usage, but cannot erase the allowance spent on an entry.
pub(super) fn reserve(
    workflow: &WorkflowRecord,
    dispatch: &ExecutionDispatch,
    envelope: &InvocationAdmissionEnvelope,
) -> Result<(AuthorityBudget, Option<String>), RuntimeError> {
    let artifact_limit = dispatch
        .task_output_bytes()
        .ok_or_else(|| denied("task artifact ceiling is absent"))?;
    let artifact_bytes = match envelope.artifact_bytes() {
        AdmissionBound::Bounded(value) => (*value).min(artifact_limit),
        AdmissionBound::NotApplicable => 0,
        AdmissionBound::Unknown => artifact_limit,
    };
    let units = bounded(envelope.input_units(), "input units")?
        .checked_add(bounded(envelope.output_units(), "output units")?)
        .ok_or_else(|| denied("unit reservation overflow"))?;
    let (cost_minor, currency) = match envelope.monetary_cost() {
        AdmissionBound::NotApplicable => (0, workflow.spending_currency.clone()),
        AdmissionBound::Unknown => {
            return Err(denied(
                "external monetary allowance has no enforceable bound",
            ));
        }
        AdmissionBound::Bounded(value) => {
            if workflow
                .spending_currency
                .as_deref()
                .is_some_and(|currency| currency != value.currency())
            {
                return Err(denied("workflow spending account cannot mix currencies"));
            }
            let minor = value
                .maximum_micros()
                .checked_add(9_999)
                .and_then(|value| value.checked_div(10_000))
                .ok_or_else(|| denied("monetary reservation overflow"))?;
            (minor, Some(value.currency().to_owned()))
        }
    };
    let invocations = envelope
        .nested_invocations()
        .map_or(Some(1), |counts| {
            counts
                .process()
                .checked_add(counts.model())
                .and_then(|value| value.checked_add(1))
        })
        .ok_or_else(|| denied("invocation reservation overflow"))?;
    let prior = workflow.committed_allowance;
    let ceiling = workflow.allowance;
    let required = dispatch.entry_authorization().request().budget;
    let reservation = AuthorityBudget {
        invocations: add(
            prior.invocations,
            invocations,
            ceiling.invocations,
            "external invocations",
        )?,
        units: add(prior.units, units, ceiling.units, "input/output units")?,
        artifact_bytes: add(
            prior.artifact_bytes,
            artifact_bytes,
            ceiling.artifact_bytes,
            "artifact bytes",
        )?,
        cost_minor: add(
            prior.cost_minor,
            cost_minor,
            ceiling.cost_minor,
            "monetary spending",
        )?,
        duration_ms: add(
            prior.duration_ms,
            required.duration_ms.unwrap_or(0),
            ceiling.duration_ms,
            "execution duration",
        )?,
        concurrency: None,
    };
    let concurrent = workflow
        .tasks
        .values()
        .filter(|task| {
            task.frozen.as_ref().is_some_and(|start| start.entered) && !task.state.is_settled()
        })
        .count();
    let concurrent = u32::try_from(concurrent)
        .ok()
        .and_then(|count| count.checked_add(1))
        .ok_or_else(|| denied("concurrency reservation overflow"))?;
    if ceiling
        .concurrency
        .is_none_or(|maximum| concurrent > maximum)
    {
        return Err(denied(
            "concurrent external execution allowance exhausted or absent",
        ));
    }
    Ok((reservation, currency))
}

fn bounded(bound: &AdmissionBound<u64>, name: &str) -> Result<u64, RuntimeError> {
    match bound {
        AdmissionBound::Bounded(value) => Ok(*value),
        AdmissionBound::NotApplicable => Ok(0),
        AdmissionBound::Unknown => Err(denied(&format!(
            "external {name} have no enforceable bound"
        ))),
    }
}

fn add(
    prior: Option<u64>,
    requested: u64,
    maximum: Option<u64>,
    name: &str,
) -> Result<Option<u64>, RuntimeError> {
    if requested == 0 {
        return Ok(prior);
    }
    let total = prior
        .unwrap_or(0)
        .checked_add(requested)
        .ok_or_else(|| denied(&format!("{name} cumulative reservation overflow")))?;
    if maximum.is_none_or(|maximum| total > maximum) {
        return Err(denied(&format!(
            "{name} cumulative allowance exhausted or absent"
        )));
    }
    Ok(Some(total))
}

fn denied(message: &str) -> RuntimeError {
    RuntimeError::Scheduling(message.to_owned())
}
