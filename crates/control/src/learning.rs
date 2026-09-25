//! Compare methods against a declaration made before proposal generation.
//!
//! These documents do not execute work or create a second run ledger. The caller retains a
//! declaration through an ordinary authorized command receipt, runs its exact slots through the
//! runtime, and supplies observations read from the owning journals. Missing observations produce
//! an inconclusive result. A comparison never grants permission to publish a method.

use crate::ControlError;
use milkdrift_authority::ActorRef;
use milkdrift_blueprint::RevisionId;
use milkdrift_capability::{CapabilityId, managed::ManagedName};
use milkdrift_persistence::{CommandId, ControllerResourceBudget, ControllerResourceTotals};
use milkdrift_workspace::{ArtifactReference, CandidateEvaluation, RunId, ValueKey};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

mod request;
pub use request::{KnowledgeSelection, LearningRequest, SourcePage};

/// Current declaration and comparison format.
pub const LEARNING_SCHEMA_VERSION: u32 = 1;

/// An immutable accepted command, including after receipt archival.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LearningReceiptReference {
    /// Authenticated command owner; knowledge of this identity grants no read access.
    pub actor: ActorRef,
    /// Exact idempotency identity, never a request for the latest result.
    pub command: CommandId,
}

/// Keep controlled fixtures distinct from actual model choices throughout comparison and reuse.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LearningLane {
    /// Controlled model responses and deliberately seeded implementation failures.
    SeededFixture,
    /// Retained external model requests and actual responses.
    ObservedModel,
}

/// A caller-scoped request reserved before any evaluation results exist.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EvaluationInvocation {
    /// Authenticated client that submits the public method invocation.
    pub actor: ActorRef,
    /// Exact request key; replay resolves the same accepted child, including after archival.
    pub request: CommandId,
}

/// One preallocated invocation and its independent writable setup and verification target.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EvaluationSlot {
    /// New caller-scoped request; the serving owner supplies its actual child run identity.
    pub invocation: EvaluationInvocation,
    /// Installation whose editing claim protects this run's ordinary working files.
    pub workspace: ManagedName,
    /// Exact approved worker recipe; unrecorded scratch tools cannot qualify a maintained method.
    pub workspace_configuration: String,
    /// Worker generation fixed before evaluation.
    pub workspace_generation: u64,
    /// Private staging target; results from another slot cannot qualify this one.
    pub target: ManagedName,
    /// Exact prepared target configuration, including its product parameters.
    pub configuration: String,
    /// Exact target generation before any candidate is evaluated.
    pub generation: u64,
}

/// The same held-out input applied independently to both methods.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EvaluationPair {
    /// Stable operator-selected case name.
    pub name: String,
    /// Exact evaluator-owned input. Proposal context must not contain it.
    pub input: ArtifactReference,
    /// Baseline run and private state.
    pub baseline: EvaluationSlot,
    /// Candidate run and private state.
    pub candidate: EvaluationSlot,
}

/// A finite criterion supplied by the evaluator, independently of the candidate.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LearningDeclaration {
    /// Exact reader version.
    pub schema_version: u32,
    /// Fixture and model evidence are never silently combined.
    pub lane: LearningLane,
    /// Immutable source selection accepted before this declaration.
    pub selection: LearningReceiptReference,
    /// Current reusable method, separate from any source run's adopted repair.
    pub baseline: RevisionId,
    /// Preallocated proposal run. It must not exist when the evaluator fixes this declaration.
    pub proposal_run: RunId,
    /// Agreement that must remain unchanged in the candidate and every evaluation.
    pub agreement: String,
    /// Required protected effect policy.
    pub policy: String,
    /// Exact operator-owned verifier implementation.
    pub verifier: String,
    /// Required check names in the verifier's declared order.
    pub checks: Vec<String>,
    /// Workflow input field that receives each pair's exact held-out artifact.
    pub input_field: ValueKey,
    /// Terminal workflow field whose returned artifact must be the final verified candidate.
    pub candidate_output: ValueKey,
    /// Complete ordered comparison, fixed before proposal generation.
    pub pairs: Vec<EvaluationPair>,
    /// Per-run cumulative account ceiling; descendants and retries share the same account.
    pub allowance: ControllerResourceBudget,
    /// Maximum elapsed time for each method/input, including repairs.
    pub maximum_duration_ms: u64,
    /// Maximum candidate submissions per method/input.
    pub maximum_submissions: u16,
    /// Required reduction in total failed submissions, with no paired regression.
    pub minimum_repair_reduction: u16,
    /// Exact tool/model/recipe documents selected for this comparison.
    pub provenance: Vec<ArtifactReference>,
    /// Unknown effective settings are retained, never replaced with assumed defaults.
    pub unknowns: Vec<String>,
    /// A separate publisher may promote only this exact future generation.
    pub publication: CapabilityId,
    /// New generation; the original stays independently invocable until retired.
    pub generation: u64,
}

fn invalid(message: &str) -> ControlError {
    ControlError::InvalidContract(message.to_owned())
}
fn digest(value: &str) -> bool {
    milkdrift_contracts::is_canonical_blake3_digest(value)
}
impl LearningDeclaration {
    /// Validate finite bounds, exact identities, and independent writable state.
    pub fn validate(&self) -> Result<(), ControlError> {
        if self.schema_version != LEARNING_SCHEMA_VERSION
            || ![&self.agreement, &self.policy, &self.verifier]
                .into_iter()
                .all(|v| digest(v))
            || self.pairs.is_empty()
            || self.pairs.len() > 32
            || self.checks.is_empty()
            || self.checks.len() > 32
            || self.maximum_submissions == 0
            || self.maximum_submissions > 32
            || self.maximum_duration_ms == 0
            || self.minimum_repair_reduction == 0
            || self.generation < 2
            || self.provenance.len() > 32
            || self.unknowns.len() > 32
            || self.unknowns.iter().any(|s| s.is_empty() || s.len() > 512)
        {
            return Err(invalid(
                "invalid learning declaration version, identity or bounds",
            ));
        }
        let mut checks = BTreeSet::new();
        for check in &self.checks {
            if check.is_empty()
                || check.len() > 64
                || !checks.insert(check)
                || !check
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
            {
                return Err(invalid("invalid or repeated required learning check"));
            }
        }
        let mut names = BTreeSet::new();
        let mut inputs = BTreeSet::new();
        let mut input_digests = BTreeSet::new();
        let mut invocations = BTreeSet::new();
        let mut resources = BTreeSet::new();
        for pair in &self.pairs {
            if pair.name.is_empty()
                || pair.name.len() > 96
                || !names.insert(&pair.name)
                || !inputs.insert(pair.input.artifact())
                || !input_digests.insert(pair.input.digest())
            {
                return Err(invalid(
                    "evaluation inputs must be distinct and explicitly named",
                ));
            }
            for slot in [&pair.baseline, &pair.candidate] {
                if !invocations.insert((&slot.invocation.actor, &slot.invocation.request))
                    || !resources.insert(&slot.workspace)
                    || !resources.insert(&slot.target)
                    || !digest(&slot.configuration)
                    || !digest(&slot.workspace_configuration)
                    || slot.generation == 0
                    || slot.workspace_generation == 0
                {
                    return Err(invalid(
                        "evaluation slots must have independent invocations and writable resources",
                    ));
                }
            }
        }
        Ok(())
    }

    /// Canonical commitment retained before the model sees any source evidence.
    pub fn digest(&self) -> Result<String, ControlError> {
        self.validate()?;
        let bytes = serde_json::to_vec(&serde_json::to_value(self)?)?;
        let mut hasher = blake3::Hasher::new();
        hasher.update(b"milkdrift.learning-declaration.v1\0");
        hasher.update(&bytes);
        Ok(format!("b3_{}", hasher.finalize()))
    }
}

/// Facts read from the run, account and private verifier owners, not scores submitted by a model.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MethodEvaluation {
    /// Preallocated public request, resolved through its authoritative serving receipt.
    pub invocation: EvaluationInvocation,
    /// Actual child run retained by that receipt.
    pub run: RunId,
    /// Revision actually selected when the run was created.
    pub method: RevisionId,
    /// Input actually accepted by that run.
    pub input: ArtifactReference,
    /// Whether all declared history was available to the bounded reader.
    pub history_complete: bool,
    /// Successful terminal run, independently of a candidate's claims.
    pub succeeded: bool,
    /// Established forbidden effect or agreement violation, if any.
    pub violation: Option<String>,
    /// Elapsed time from accepted start through terminal evidence; unknown stays unknown.
    pub duration_ms: Option<u64>,
    /// Settled attributable use, absent while reservations or metering remain unresolved.
    pub usage: Option<ControllerResourceTotals>,
    /// Accepted account declaration; equality prevents a run from gaining another allowance.
    pub allowance: Option<ControllerResourceBudget>,
    /// Artifact returned in the declared terminal field, rather than merely tested along the way.
    pub output: Option<ArtifactReference>,
    /// Every candidate submission in accepted order, including incomplete/failed checks.
    pub submissions: Vec<CandidateEvaluation>,
}

/// Result of one pair, retained even when it prevents promotion.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LearningPairResult {
    /// Exact case name from the declaration.
    pub name: String,
    /// Failed submissions before the final accepted candidate, when established.
    pub baseline_repairs: Option<u16>,
    /// Same metric for the candidate.
    pub candidate_repairs: Option<u16>,
}

/// A complete comparison may establish eligibility, rejection, or insufficient evidence.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LearningOutcome {
    /// The finite criterion passed; separate current publication authority is still required.
    Eligible,
    /// Established failed obligations, a paired regression, or an insufficient complete reduction.
    Rejected,
    /// Missing comparison, unresolved effects/usage, or too little baseline rework to compare.
    Inconclusive,
}

/// Comparison evidence is immutable; publication is a separate consequential command.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LearningComparison {
    /// Commitment to the predeclared inputs, criteria and ceilings.
    pub declaration: String,
    /// Exact current method.
    pub baseline: RevisionId,
    /// Exact proposed method.
    pub candidate: RevisionId,
    /// Evidence lane retained through promotion/rejection.
    pub lane: LearningLane,
    /// Observed bounded comparison.
    pub pairs: Vec<LearningPairResult>,
    /// One of the three explicit outcomes.
    pub outcome: LearningOutcome,
    /// Bounded reasons, including unavailable evidence rather than fabricated zero failures.
    pub reasons: Vec<String>,
}

enum Measurement {
    Complete(u16),
    Unknown,
    Failed,
}

fn measure(
    declaration: &LearningDeclaration,
    slot: &EvaluationSlot,
    method: &RevisionId,
    input: &ArtifactReference,
    observed: &MethodEvaluation,
) -> Result<Measurement, ControlError> {
    if observed.invocation != slot.invocation
        || &observed.method != method
        || &observed.input != input
    {
        return Err(invalid(
            "evaluation does not match its predeclared invocation, method and input",
        ));
    }
    if observed.violation.is_some()
        || observed.submissions.len() > usize::from(declaration.maximum_submissions)
    {
        return Ok(Measurement::Failed);
    }
    let mut identities = BTreeSet::new();
    let mut previous_start = 0;
    let mut repairs = 0;
    let mut final_pass = false;
    let mut unknown = false;
    for (index, evaluation) in observed.submissions.iter().enumerate() {
        evaluation
            .validate()
            .map_err(|error| invalid(&error.to_string()))?;
        let subject = &evaluation.subject;
        if subject.target != slot.target
            || subject.configuration != slot.configuration
            || subject.generation != slot.generation
            || subject.agreement != declaration.agreement
            || subject.policy != declaration.policy
            || subject.verifier != declaration.verifier
            || evaluation
                .checks
                .iter()
                .map(|c| &c.name)
                .ne(declaration.checks.iter())
            || !identities.insert(&evaluation.identity)
            || evaluation.started_at < previous_start
        {
            return Err(invalid(
                "verification differs from the fixed target, verifier or check set",
            ));
        }
        previous_start = evaluation.started_at;
        if !evaluation.complete || evaluation.checks.iter().any(|c| c.passed.is_none()) {
            unknown = true;
            final_pass = false;
            continue;
        }
        final_pass = evaluation.checks.iter().all(|c| c.passed == Some(true));
        if !final_pass && index + 1 < observed.submissions.len() {
            repairs += 1;
        }
    }
    if observed.history_complete
        && observed
            .submissions
            .last()
            .is_some_and(|e| e.complete && e.checks.iter().all(|c| c.passed.is_some()))
        && !final_pass
    {
        return Ok(Measurement::Failed);
    }
    if observed.history_complete
        && observed.succeeded
        && final_pass
        && observed.submissions.last().is_some_and(|evaluation| {
            observed.output.as_ref() != Some(&evaluation.subject.artifact)
        })
    {
        return Ok(Measurement::Failed);
    }
    if observed
        .duration_ms
        .is_some_and(|duration| duration > declaration.maximum_duration_ms)
        || observed
            .allowance
            .as_ref()
            .is_some_and(|allowance| allowance != &declaration.allowance)
    {
        return Ok(Measurement::Failed);
    }
    let budget = &declaration.allowance;
    if observed.usage.as_ref().is_some_and(|usage| {
        usage.cost_micros() > budget.cost_micros()
            || usage.input_units() > budget.input_units()
            || usage.output_units() > budget.output_units()
            || usage.artifact_bytes() > budget.artifact_bytes()
            || usage.process_admissions() > budget.process_admissions()
            || usage.model_admissions() > budget.model_admissions()
    }) {
        return Ok(Measurement::Failed);
    }
    if unknown
        || observed.duration_ms.is_none()
        || observed.usage.is_none()
        || observed.allowance.is_none()
        || !observed.history_complete
        || !observed.succeeded
        || !final_pass
    {
        return Ok(Measurement::Unknown);
    }
    Ok(Measurement::Complete(repairs))
}

/// Assess the complete preallocated comparison. The caller must obtain observations through
/// authorized journal reads; deserializing an uploaded `MethodEvaluation` does not authenticate it.
/// An unaccepted reserved invocation is `None`, which preserves missing work as inconclusive.
/// No result can promote a method or edit the declaration, verifier, account or earlier run.
pub fn compare_methods(
    declaration: &LearningDeclaration,
    candidate: &RevisionId,
    observations: &[(Option<MethodEvaluation>, Option<MethodEvaluation>)],
) -> Result<LearningComparison, ControlError> {
    declaration.validate()?;
    if observations.len() != declaration.pairs.len() || candidate == &declaration.baseline {
        return Err(invalid(
            "comparison requires every declared pair and a distinct candidate revision",
        ));
    }
    let mut result = LearningComparison {
        declaration: declaration.digest()?,
        baseline: declaration.baseline.clone(),
        candidate: candidate.clone(),
        lane: declaration.lane,
        pairs: Vec::new(),
        outcome: LearningOutcome::Inconclusive,
        reasons: Vec::new(),
    };
    let (mut baseline_total, mut candidate_total) = (0_u32, 0_u32);
    let (mut rejected, mut unknown) = (false, false);
    let mut runs = BTreeSet::new();
    for (pair, (baseline, proposed)) in declaration.pairs.iter().zip(observations) {
        for observed in baseline.iter().chain(proposed.iter()) {
            if !runs.insert(&observed.run) || observed.run == declaration.proposal_run {
                return Err(invalid(
                    "evaluation invocations must resolve to distinct new runs",
                ));
            }
        }
        let a = baseline
            .as_ref()
            .map(|baseline| {
                measure(
                    declaration,
                    &pair.baseline,
                    &declaration.baseline,
                    &pair.input,
                    baseline,
                )
            })
            .transpose()?
            .unwrap_or(Measurement::Unknown);
        let b = proposed
            .as_ref()
            .map(|proposed| {
                measure(
                    declaration,
                    &pair.candidate,
                    candidate,
                    &pair.input,
                    proposed,
                )
            })
            .transpose()?
            .unwrap_or(Measurement::Unknown);
        let baseline_repairs = if let Measurement::Complete(n) = a {
            baseline_total += u32::from(n);
            Some(n)
        } else {
            None
        };
        let candidate_repairs = if let Measurement::Complete(n) = b {
            candidate_total += u32::from(n);
            Some(n)
        } else {
            None
        };
        if matches!(b, Measurement::Failed)
            || baseline_repairs
                .zip(candidate_repairs)
                .is_some_and(|(a, b)| b > a)
        {
            rejected = true;
            result.reasons.push(format!(
                "{}: candidate failed obligations, exceeded its envelope or regressed",
                pair.name
            ));
        }
        if baseline_repairs.is_none() || candidate_repairs.is_none() {
            unknown = true;
            result.reasons.push(format!(
                "{}: complete comparable evidence is unavailable",
                pair.name
            ));
        }
        result.pairs.push(LearningPairResult {
            name: pair.name.clone(),
            baseline_repairs,
            candidate_repairs,
        });
    }
    let threshold = u32::from(declaration.minimum_repair_reduction);
    result.outcome = if rejected {
        LearningOutcome::Rejected
    } else if unknown {
        LearningOutcome::Inconclusive
    } else if baseline_total < threshold {
        result
            .reasons
            .push("baseline has insufficient rework for the declared threshold".into());
        LearningOutcome::Inconclusive
    } else if baseline_total.saturating_sub(candidate_total) >= threshold {
        LearningOutcome::Eligible
    } else {
        result
            .reasons
            .push("complete comparison did not reach the declared improvement threshold".into());
        LearningOutcome::Rejected
    };
    Ok(result)
}
