//! Purpose-specific checks over immutable outputs. The ordinary control capability publishes
//! the decision; an ordinary branch controls continuation. These checks perform no external work.

use std::collections::{BTreeMap, BTreeSet};

use milkdrift_capability::ArtifactReference;
use milkdrift_model::{FinishReason, ModelResponseDocument, Usage};
use serde::{Deserialize, Serialize};

use crate::ControlError;

mod blueprint;
#[cfg(test)]
mod tests;
pub use blueprint::{result_acceptance_gate, result_acceptance_task};

/// Supported acceptance contract and decision schema version.
pub const RESULT_ACCEPTANCE_SCHEMA_VERSION: u32 = 1;

/// Input containing the acceptance contract for an ordinary workflow-control task.
pub const RESULT_ACCEPTANCE_INPUT: &str = "milkdrift.acceptance";
/// Named artifact containing the decision, including a rejection reason.
pub const RESULT_ACCEPTANCE_OUTPUT: &str = "acceptance_result";
/// Published only when the declared requirement is satisfied. Branch on its presence.
pub const ACCEPTED_RESULT_OUTPUT: &str = "accepted_result";
/// Maximum bytes read from any one result or evidence artifact.
pub const MAX_ACCEPTANCE_INPUT_BYTES: u64 = milkdrift_model::MAX_MODEL_DOCUMENT_BYTES as u64;

/// Declares what an output must establish before dependent work may proceed.
///
/// Supply this contract as [`RESULT_ACCEPTANCE_INPUT`] to `workflow.accept_result`, with
/// the upstream artifact on `result` and each required evidence artifact on its named
/// input. The operation publishes a decision on every evaluated result and publishes
/// [`ACCEPTED_RESULT_OUTPUT`] only on acceptance. Use an ordinary artifact-presence
/// branch to route rejection to a hold, failure, or prospective remediation.
/// A prose check establishes mechanical completeness, not the correctness of a review.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ResultAcceptanceContract {
    schema_version: u32,
    requirement: ResultRequirement,
    evidence_inputs: BTreeSet<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ContractWire {
    schema_version: u32,
    requirement: ResultRequirement,
    evidence_inputs: BTreeSet<String>,
}

milkdrift_contracts::deserialize_via!(ResultAcceptanceContract, ContractWire, |wire| {
    if wire.schema_version != RESULT_ACCEPTANCE_SCHEMA_VERSION {
        Err(invalid_contract())
    } else {
        Self::new(wire.requirement, wire.evidence_inputs)
    }
});

impl ResultAcceptanceContract {
    /// Constructs a bounded requirement. Evidence input names must be distinct from reserved inputs.
    pub fn new(
        requirement: ResultRequirement,
        evidence_inputs: BTreeSet<String>,
    ) -> Result<Self, ControlError> {
        if evidence_inputs.len() > 32
            || evidence_inputs
                .iter()
                .any(|name| !safe_name(name) || name == "result" || name == RESULT_ACCEPTANCE_INPUT)
        {
            return Err(invalid_contract());
        }
        match &requirement {
            ResultRequirement::ModelTools { names }
                if names.is_empty()
                    || names.len() > 64
                    || names.iter().any(|name| !safe_name(name)) =>
            {
                return Err(invalid_contract());
            }
            ResultRequirement::Verification { checks }
                if checks.is_empty()
                    || checks.len() > 64
                    || checks.iter().any(|name| !safe_name(name)) =>
            {
                return Err(invalid_contract());
            }
            ResultRequirement::ModelDecision | ResultRequirement::ReviewDecision
                if evidence_inputs.is_empty() =>
            {
                return Err(invalid_contract());
            }
            _ => {}
        }
        Ok(Self {
            schema_version: RESULT_ACCEPTANCE_SCHEMA_VERSION,
            requirement,
            evidence_inputs,
        })
    }

    /// Requirement whose meaning is frozen in the workflow revision.
    #[must_use]
    pub const fn requirement(&self) -> &ResultRequirement {
        &self.requirement
    }

    /// Declared artifact inputs whose exact references a structured decision must cite.
    #[must_use]
    pub const fn evidence_inputs(&self) -> &BTreeSet<String> {
        &self.evidence_inputs
    }

    pub(crate) fn evaluate(
        &self,
        bytes: &[u8],
        evidence: &[ArtifactReference],
    ) -> ResultAcceptance {
        let mut result = ResultAcceptance::rejected(AcceptanceReason::InvalidStructure);
        result.requirement = Some(self.requirement.clone());
        match &self.requirement {
            ResultRequirement::ModelProse
            | ResultRequirement::ModelDecision
            | ResultRequirement::ModelTools { .. } => {
                let Ok(document) = ModelResponseDocument::from_json(bytes) else {
                    return result;
                };
                let response = document.body();
                result.model = Some(AcceptanceModelDiagnostics {
                    finish_reason: response.finish_reason(),
                    usage: response.usage().clone(),
                });
                if response.finish_reason() == FinishReason::Length {
                    result.reason = AcceptanceReason::OutputAllowanceExhausted;
                    return result;
                }
                match &self.requirement {
                    ResultRequirement::ModelTools { names } => {
                        if response.finish_reason() != FinishReason::ToolCalls {
                            result.reason = AcceptanceReason::IncompleteResult;
                        } else if response.tool_calls().is_empty()
                            || response
                                .tool_calls()
                                .iter()
                                .any(|call| !names.contains(call.name()))
                        {
                            result.reason = AcceptanceReason::MissingRequiredOutput;
                        } else {
                            result.accept();
                        }
                    }
                    ResultRequirement::ModelProse | ResultRequirement::ModelDecision => {
                        if response.finish_reason() != FinishReason::Stop {
                            result.reason = AcceptanceReason::IncompleteResult;
                        } else if matches!(self.requirement, ResultRequirement::ModelProse) {
                            if response.text().trim().is_empty() {
                                result.reason = AcceptanceReason::MissingRequiredOutput;
                            } else {
                                result.accept();
                            }
                        } else if let Some(value) = response.structured() {
                            result.decision(value.value().clone(), evidence);
                        }
                    }
                    _ => {}
                }
            }
            ResultRequirement::ReviewProse => {
                if std::str::from_utf8(bytes).is_ok_and(|text| !text.trim().is_empty()) {
                    result.accept();
                } else {
                    result.reason = AcceptanceReason::MissingRequiredOutput;
                }
            }
            ResultRequirement::ReviewDecision => {
                if let Ok(value) = milkdrift_contracts::parse_json_without_duplicates(bytes) {
                    result.decision(value, evidence);
                }
            }
            ResultRequirement::Verification { checks } => {
                if let Ok(report) = milkdrift_contracts::parse_json_without_duplicates(bytes)
                    .and_then(serde_json::from_value::<VerifiedCheckpoint>)
                {
                    if report.checkpoint.is_empty()
                        || report.checkpoint.len() > 256
                        || report.checked_checkpoint != report.checkpoint
                    {
                        result.reason = AcceptanceReason::StaleCheckpoint;
                    } else if report.checks.len() > 64
                        || checks
                            .iter()
                            .any(|check| report.checks.get(check) != Some(&true))
                    {
                        result.reason = AcceptanceReason::VerificationFailed;
                    } else if let CodingResult::NoChange { justification } = &report.coding {
                        if justification.trim().is_empty() || justification.len() > 4096 {
                            result.reason = AcceptanceReason::UnverifiedDecision;
                        } else {
                            result.accept();
                        }
                    } else {
                        result.accept();
                    }
                    if !report.checkpoint.is_empty() && report.checkpoint.len() <= 256 {
                        result.checkpoint = Some(report.checkpoint);
                    }
                }
            }
        }
        result
    }
}

/// Fixed purpose choices; generic model tasks receive no implicit acceptance policy.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ResultRequirement {
    /// Naturally completed, non-whitespace final model text. Does not judge its quality.
    ModelProse,
    /// Naturally completed structured review decision citing all required evidence.
    ModelDecision,
    /// Tool-only output requesting declared tools; calls remain unexecuted data.
    ModelTools {
        /// Permitted returned tool names.
        names: BTreeSet<String>,
    },
    /// A non-whitespace UTF-8 review artifact from a configured reviewer.
    ReviewProse,
    /// A structured review artifact citing the required exact evidence.
    ReviewDecision,
    /// A configured verifier's report of checks on one unchanged checkpoint.
    Verification {
        /// Checks that must all pass on that checkpoint.
        checks: BTreeSet<String>,
    },
}

/// A verifier's evidence binds its checks to the repository state observed before and after them.
/// The configured verifier owns reading the repository and running these named checks.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct VerifiedCheckpoint {
    /// Exact repository state before the configured checks.
    pub checkpoint: String,
    /// Exact repository state after the checks; a mismatch refuses acceptance.
    pub checked_checkpoint: String,
    /// Results of the configured checks, not arbitrary model confidence.
    pub checks: BTreeMap<String, bool>,
    /// Verified change or an explicit justified absence of changes.
    pub coding: CodingResult,
}

/// Coding outcomes reported by the separate repository verifier.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum CodingResult {
    /// The configured verifier established the requested change at the checkpoint.
    Changed,
    /// The verifier established that no change was needed.
    NoChange {
        /// Bounded explanation for the explicit no-change result.
        justification: String,
    },
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReviewDecision {
    accepted: bool,
    evidence: Vec<ArtifactReference>,
}

/// Durable acceptance evidence, separate from the source invocation's immutable outcome.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ResultAcceptance {
    /// Version of the result contract.
    pub schema_version: u32,
    /// Declared purpose, absent when the request could not be read safely.
    pub requirement: Option<ResultRequirement>,
    /// Whether the required output was established.
    pub accepted: bool,
    /// Closed reason vocabulary contains no source content or protected identities.
    pub reason: AcceptanceReason,
    /// Authorized model finish and usage observations when a response was readable.
    pub model: Option<AcceptanceModelDiagnostics>,
    /// Checkpoint examined by the authorized verifier, when readable.
    pub checkpoint: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AcceptanceWire {
    schema_version: u32,
    requirement: Option<ResultRequirement>,
    accepted: bool,
    reason: AcceptanceReason,
    model: Option<AcceptanceModelDiagnostics>,
    checkpoint: Option<String>,
}

milkdrift_contracts::deserialize_via!(ResultAcceptance, AcceptanceWire, |wire| {
    if wire.schema_version != RESULT_ACCEPTANCE_SCHEMA_VERSION
        || wire.accepted != (wire.reason == AcceptanceReason::Accepted)
        || (wire.accepted && wire.requirement.is_none())
        || wire
            .checkpoint
            .as_ref()
            .is_some_and(|checkpoint| checkpoint.is_empty() || checkpoint.len() > 256)
    {
        Err(invalid_contract())
    } else {
        Ok(Self {
            schema_version: wire.schema_version,
            requirement: wire.requirement,
            accepted: wire.accepted,
            reason: wire.reason,
            model: wire.model,
            checkpoint: wire.checkpoint,
        })
    }
});

impl ResultAcceptance {
    pub(crate) const fn rejected(reason: AcceptanceReason) -> Self {
        Self {
            schema_version: RESULT_ACCEPTANCE_SCHEMA_VERSION,
            requirement: None,
            accepted: false,
            reason,
            model: None,
            checkpoint: None,
        }
    }
    fn accept(&mut self) {
        self.accepted = true;
        self.reason = AcceptanceReason::Accepted;
    }
    fn decision(&mut self, value: serde_json::Value, evidence: &[ArtifactReference]) {
        let Ok(decision) = serde_json::from_value::<ReviewDecision>(value) else {
            return;
        };
        if decision.evidence.len() > 32
            || evidence
                .iter()
                .any(|reference| !decision.evidence.contains(reference))
            || decision
                .evidence
                .iter()
                .any(|reference| !evidence.contains(reference))
        {
            self.reason = AcceptanceReason::UnverifiedDecision;
        } else if !decision.accepted {
            self.reason = AcceptanceReason::ReviewRejected;
        } else {
            self.accept();
        }
    }
}

/// Safe reason for an acceptance result; raw output stays in its separately authorized artifact.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AcceptanceReason {
    /// Declared requirement passed.
    Accepted,
    /// A required output was absent or whitespace-only.
    MissingRequiredOutput,
    /// Source data did not satisfy its typed contract.
    InvalidStructure,
    /// Provider stopped at the requested output allowance.
    OutputAllowanceExhausted,
    /// Provider did not report the completion required for this output shape.
    IncompleteResult,
    /// Evidence was unavailable or outside the caller's authority.
    EvidenceUnavailable,
    /// The repository changed during verification.
    StaleCheckpoint,
    /// A configured check did not pass.
    VerificationFailed,
    /// Required evidence or a justified decision was missing.
    UnverifiedDecision,
    /// The configured reviewer returned a negative decision.
    ReviewRejected,
}

/// Provider observations retained when acceptance rejects a generated result.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AcceptanceModelDiagnostics {
    /// Provider-neutral stop reason.
    pub finish_reason: FinishReason,
    /// Returned usage; absent dimensions remain unknown.
    pub usage: Usage,
}

fn safe_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.' | b':' | b'/')
        })
}

fn invalid_contract() -> ControlError {
    ControlError::InvalidContract("invalid bounded result acceptance contract".to_owned())
}
