use std::collections::{BTreeMap, BTreeSet};

use milkdrift_capability::{
    ArtifactReference, CapabilityId, ExecutionTrustClass, ProviderProfileRef, SideEffectClass,
};
use milkdrift_contracts::{JsonBoundKind, JsonLimits};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

use crate::{
    MAX_INLINE_PROMPT_BYTES, MAX_PROMPT_SEQUENCE_DOCUMENT_BYTES, MAX_PROMPT_SEQUENCE_STAGES,
    MAX_TOTAL_INLINE_PROMPT_BYTES,
};

/// Current prompt-sequence import schema.
pub const PROMPT_SEQUENCE_SCHEMA_VERSION_V2: u32 = 2;
const MAX_DOCUMENT_DEPTH: usize = 48;
const MAX_CONTAINER_ITEMS: usize = 4_096;
const MAX_EXTENSIONS: usize = 32;
const MAX_DECLARED_OUTPUTS: usize = 32;
const MAX_ALLOWED_PATHS: usize = 128;
const MAX_PROFILE_CHECKS: usize = 64;
const DOCUMENT_LIMITS: JsonLimits = JsonLimits {
    maximum_depth: MAX_DOCUMENT_DEPTH,
    maximum_string_bytes: MAX_INLINE_PROMPT_BYTES,
    maximum_key_bytes: 192,
    maximum_container_items: MAX_CONTAINER_ITEMS,
};

/// Invalid, hostile, or semantically incomplete sequence input.
#[derive(Debug, Error)]
pub enum PromptSequenceError {
    /// JSON syntax, duplicates, or typed shape were invalid.
    #[error("invalid prompt-sequence document: {0}")]
    Json(String),
    /// A future schema cannot be interpreted safely.
    #[error("unsupported prompt-sequence schema version {found}; supported version is 2")]
    UnsupportedVersion {
        /// Observed schema version.
        found: u32,
    },
    /// A defensive document or field bound was exceeded.
    #[error("prompt-sequence bound exceeded at {location}: {reason}")]
    Bounds {
        /// JSON-like source location.
        location: String,
        /// Stable bound summary.
        reason: String,
    },
    /// A private product invariant was violated.
    #[error("invalid prompt sequence: {0}")]
    Invalid(String),
    /// Markdown envelope or prompt sections were invalid.
    #[error("invalid prompt-sequence Markdown: {0}")]
    Markdown(String),
    /// Ordinary blueprint construction or validation failed.
    #[error("prompt-sequence blueprint compilation failed: {0}")]
    Compilation(String),
}

/// Fresh versus explicit continuation behavior for one stage.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionPolicy {
    /// Start a new external process/provider context for the stage.
    #[default]
    Fresh,
    /// Continue only through an exact context artifact selected by policy.
    ExplicitContinuation,
}

/// Reference to one preconfigured capability/profile generation selector.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CapabilityProfileRef {
    /// Exact capability identity.
    pub capability: CapabilityId,
    /// Exact namespaced operation advertised by the capability.
    pub operation: milkdrift_capability::OperationId,
    /// Optional opaque configured provider/process profile reference.
    pub provider_profile: Option<ProviderProfileRef>,
    /// Exact process-isolation/trust class required from resolution.
    pub execution_trust: ExecutionTrustClass,
    /// Highest acceptable advertised side effect.
    pub maximum_side_effect: SideEffectClass,
}

/// Prompt bytes or an exact already-published content-addressed artifact.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case", tag = "type", deny_unknown_fields)]
pub enum PromptSource {
    /// Inline Markdown retained immutably in the generated revision.
    InlineMarkdown {
        /// Exact untrusted prompt bytes.
        content: String,
    },
    /// Exact immutable artifact published through an authorized capability.
    Artifact {
        /// Digest-, media-type-, and size-bound reference.
        reference: ArtifactReference,
    },
}

/// Named declarative verification checks interpreted only by the configured verifier.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct VerificationContract {
    /// Exact preconfigured verification profile.
    pub profile: CapabilityProfileRef,
    /// Bounded safe check identities, never shell source.
    pub checks: Vec<String>,
    /// Artifact name whose presence is the success fact.
    pub success_artifact: String,
    /// Required structured evidence artifact name.
    pub result_artifact: String,
    /// Optional bounded log artifact name.
    pub log_artifact: Option<String>,
}

/// Action after verification does not publish its declared success artifact.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FailurePolicy {
    /// Route to review and a durable approval signal wait.
    PauseForReview,
    /// End at an explicit failure terminal.
    FailRun,
}

/// Authority path required before work may continue after review.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalPolicy {
    /// An authorized actor must approve/apply a proposal and deliver the exact signal.
    SharedControlPath,
}

/// One declared coding-agent output artifact.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DeclaredOutput {
    /// Safe output name matching the configured capability profile.
    pub name: String,
    /// Exact media type expected from the capability contract.
    pub media_type: String,
    /// Whether absence makes the capability invocation fail.
    pub required: bool,
}

/// One ordered implementation prompt and its exact execution policy.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StageDefinition {
    /// Stable stage identity.
    pub id: String,
    /// Operator-facing title.
    pub title: String,
    /// Exact prompt content/reference.
    pub prompt: PromptSource,
    /// Fresh or exact explicit continuation behavior.
    pub session: SessionPolicy,
    /// Preconfigured coding-agent capability/profile.
    pub coding: CapabilityProfileRef,
    /// Preconfigured verification capability and data-only contract.
    pub verification: VerificationContract,
    /// Verification failure behavior.
    pub failure: FailurePolicy,
    /// Reviewer/controller capability used on the failure path.
    pub reviewer: CapabilityProfileRef,
    /// Shared control-plane approval requirement.
    pub approval: ApprovalPolicy,
    /// Named context policy reference retained in provenance.
    pub context_policy_ref: String,
    /// Optional capability output declarations.
    pub outputs: Vec<DeclaredOutput>,
}

/// Allowed repository operation class declared by the operator profile.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RepositoryOperation {
    /// Read files and metadata.
    Read,
    /// Modify allowed paths.
    Write,
    /// Execute the preconfigured coding/verification capability.
    Execute,
    /// Use an operator-configured VCS capability.
    VersionControl,
    /// Use an operator-configured remote capability.
    Remote,
}

/// Operator decision for pre-existing repository changes.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DirtyTreePolicy {
    /// Reject before work when a configured repository check reports changes.
    Reject,
    /// Preserve and record the starting state as explicit evidence.
    AllowRecorded,
}

/// Sequential/parallel repository isolation strategy.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RepositoryIsolation {
    /// Ordered prompts share one authorized repository workspace.
    SharedSequential,
    /// Each branch is materialized by an operator-configured worktree capability.
    IsolatedWorktrees,
}

/// Retention/cleanup choice for materialized repository workspaces.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RepositoryCleanupPolicy {
    /// Retain accepted workspace state; clean only temporary invocation materialization.
    RetainAccepted,
    /// A configured cleanup capability may remove unaccepted isolated scopes.
    CleanupUnaccepted,
}

/// Repository input/output evidence declaration.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RepositoryArtifactPolicy {
    /// Require a starting-state artifact/reference.
    pub require_starting_state: bool,
    /// Require a diff artifact from coding/remediation tasks.
    pub require_diff: bool,
    /// Require verification result and bounded logs.
    pub require_verification_evidence: bool,
}

/// Operator-facing persistent repository workspace profile reference and policy.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RepositoryWorkspaceProfile {
    /// Stable configured workspace identity.
    pub id: String,
    /// Opaque configured root reference; never interpreted as executable input.
    pub root_ref: String,
    /// Optional immutable starting VCS revision/reference.
    pub starting_revision: Option<String>,
    /// Relative allowlist interpreted by configured repository/process capabilities.
    pub allowed_paths: Vec<String>,
    /// Declared operation classes.
    pub allowed_operations: BTreeSet<RepositoryOperation>,
    /// Dirty starting-state behavior.
    pub dirty_tree: DirtyTreePolicy,
    /// Sequential/parallel materialization strategy.
    pub isolation: RepositoryIsolation,
    /// Accepted/unaccepted workspace cleanup behavior.
    pub cleanup: RepositoryCleanupPolicy,
    /// Artifact evidence policy.
    pub artifacts: RepositoryArtifactPolicy,
    /// Opaque credential references, never secret values.
    pub credential_refs: Vec<String>,
    /// Opaque configured remote-access profile references.
    pub remote_access_refs: Vec<String>,
}

/// Sequence-wide prospective-remediation limit.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PromptSequenceBudget {
    /// Maximum reviewer/remediation generations accepted by the proposal builder.
    pub max_review_loops: u16,
}

/// Fully decoded bounded sequence body.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PromptSequence {
    /// Stable sequence import identity.
    pub id: String,
    /// Operator-facing title.
    pub title: String,
    /// Workflow lineage created by this import.
    pub workflow_id: String,
    /// Persistent repository workspace policy/reference.
    pub repository: RepositoryWorkspaceProfile,
    /// Ordered implementation stages.
    pub stages: Vec<StageDefinition>,
    /// Sequence-wide controller/revision/evidence limits.
    pub budget: PromptSequenceBudget,
    /// Bounded namespaced data-only extensions.
    pub extensions: BTreeMap<String, Value>,
}

/// Versioned portable prompt-sequence envelope.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PromptSequenceDocument {
    /// Exact document schema.
    pub schema_version: u32,
    /// Bounded ordered sequence.
    pub sequence: PromptSequence,
}

impl PromptSequenceDocument {
    /// Reads JSON or the bounded Markdown envelope, then validates all private invariants.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, PromptSequenceError> {
        if bytes.len() > MAX_PROMPT_SEQUENCE_DOCUMENT_BYTES {
            return Err(PromptSequenceError::Bounds {
                location: "$".to_owned(),
                reason: format!("document exceeds {MAX_PROMPT_SEQUENCE_DOCUMENT_BYTES} bytes"),
            });
        }
        let first = bytes
            .iter()
            .copied()
            .find(|byte| !byte.is_ascii_whitespace());
        if first == Some(b'{') {
            Self::from_json(bytes)
        } else {
            crate::markdown::parse(bytes)
        }
    }

    /// Reads duplicate-safe bounded JSON and validates semantic constraints.
    pub fn from_json(bytes: &[u8]) -> Result<Self, PromptSequenceError> {
        if bytes.len() > MAX_PROMPT_SEQUENCE_DOCUMENT_BYTES {
            return Err(PromptSequenceError::Bounds {
                location: "$".to_owned(),
                reason: format!("document exceeds {MAX_PROMPT_SEQUENCE_DOCUMENT_BYTES} bytes"),
            });
        }
        milkdrift_contracts::preflight_json_structure(bytes, DOCUMENT_LIMITS).map_err(map_bound)?;
        let value = milkdrift_contracts::parse_json_without_duplicates(bytes)
            .map_err(|error| PromptSequenceError::Json(error.to_string()))?;
        milkdrift_contracts::validate_json_value(&value, DOCUMENT_LIMITS).map_err(map_bound)?;
        Self::from_value(value)
    }

    pub(crate) fn from_value(value: Value) -> Result<Self, PromptSequenceError> {
        let version = value
            .get("schema_version")
            .and_then(Value::as_u64)
            .and_then(|value| u32::try_from(value).ok())
            .ok_or_else(|| {
                PromptSequenceError::Json("missing numeric schema_version".to_owned())
            })?;
        if version != PROMPT_SEQUENCE_SCHEMA_VERSION_V2 {
            return Err(PromptSequenceError::UnsupportedVersion { found: version });
        }
        let document: Self = serde_json::from_value(value)
            .map_err(|error| PromptSequenceError::Json(error.to_string()))?;
        document.validate()?;
        Ok(document)
    }

    pub(crate) fn preflight_markdown_header(bytes: &[u8]) -> Result<(), PromptSequenceError> {
        milkdrift_contracts::preflight_json_structure(bytes, DOCUMENT_LIMITS).map_err(map_bound)
    }

    /// Recursively key-sorted canonical JSON used for import provenance.
    pub fn to_canonical_json(&self) -> Result<Vec<u8>, PromptSequenceError> {
        let bytes = milkdrift_contracts::canonical_json_bytes(self, DOCUMENT_LIMITS)
            .map_err(|error| PromptSequenceError::Json(format!("{error:?}")))?;
        if bytes.len() > MAX_PROMPT_SEQUENCE_DOCUMENT_BYTES {
            return Err(PromptSequenceError::Bounds {
                location: "$".to_owned(),
                reason: format!("document exceeds {MAX_PROMPT_SEQUENCE_DOCUMENT_BYTES} bytes"),
            });
        }
        Ok(bytes)
    }

    /// Exact validated sequence body.
    #[must_use]
    pub const fn sequence(&self) -> &PromptSequence {
        &self.sequence
    }

    fn validate(&self) -> Result<(), PromptSequenceError> {
        if self.schema_version != PROMPT_SEQUENCE_SCHEMA_VERSION_V2 {
            return Err(PromptSequenceError::UnsupportedVersion {
                found: self.schema_version,
            });
        }
        validate_identity("sequence.id", &self.sequence.id, 128)?;
        validate_text("sequence.title", &self.sequence.title, 1, 256)?;
        validate_identity("sequence.workflow_id", &self.sequence.workflow_id, 128)?;
        validate_repository(&self.sequence.repository)?;
        validate_sequence_budget(self.sequence.budget)?;
        validate_extensions(&self.sequence.extensions)?;
        if self.sequence.stages.is_empty()
            || self.sequence.stages.len() > MAX_PROMPT_SEQUENCE_STAGES
        {
            return Err(PromptSequenceError::Bounds {
                location: "sequence.stages".to_owned(),
                reason: format!("stage count must be between 1 and {MAX_PROMPT_SEQUENCE_STAGES}"),
            });
        }
        let mut ids = BTreeSet::new();
        let mut prompt_bytes = 0_usize;
        for (index, stage) in self.sequence.stages.iter().enumerate() {
            validate_stage(stage, index)?;
            if !ids.insert(&stage.id) {
                return Err(PromptSequenceError::Invalid(format!(
                    "duplicate stage identity '{}'",
                    stage.id
                )));
            }
            if let PromptSource::InlineMarkdown { content } = &stage.prompt {
                prompt_bytes = prompt_bytes.saturating_add(content.len());
            }
            if self.sequence.repository.artifacts.require_diff
                && !stage.outputs.iter().any(|output| output.name == "diff")
            {
                return Err(PromptSequenceError::Invalid(format!(
                    "stage '{}' must declare the required diff output",
                    stage.id
                )));
            }
        }
        if prompt_bytes > MAX_TOTAL_INLINE_PROMPT_BYTES {
            return Err(PromptSequenceError::Bounds {
                location: "sequence.stages.prompt".to_owned(),
                reason: format!(
                    "aggregate inline prompt bytes exceed {MAX_TOTAL_INLINE_PROMPT_BYTES}"
                ),
            });
        }
        Ok(())
    }
}

fn validate_stage(stage: &StageDefinition, index: usize) -> Result<(), PromptSequenceError> {
    let location = format!("sequence.stages[{index}]");
    validate_identity(&format!("{location}.id"), &stage.id, 64)?;
    validate_text(&format!("{location}.title"), &stage.title, 1, 256)?;
    match &stage.prompt {
        PromptSource::InlineMarkdown { content } => {
            validate_text(
                &format!("{location}.prompt.content"),
                content,
                1,
                MAX_INLINE_PROMPT_BYTES,
            )?;
        }
        PromptSource::Artifact { reference: _ } => {}
    }
    validate_profile(&stage.coding)?;
    validate_profile(&stage.verification.profile)?;
    validate_profile(&stage.reviewer)?;
    if stage.verification.checks.is_empty() || stage.verification.checks.len() > MAX_PROFILE_CHECKS
    {
        return Err(PromptSequenceError::Bounds {
            location: format!("{location}.verification.checks"),
            reason: format!("must contain 1..={MAX_PROFILE_CHECKS} safe check identities"),
        });
    }
    for check in &stage.verification.checks {
        validate_namespaced_data_id("verification.check", check, 128)?;
    }
    validate_safe_name(
        "verification.success_artifact",
        &stage.verification.success_artifact,
    )?;
    validate_safe_name(
        "verification.result_artifact",
        &stage.verification.result_artifact,
    )?;
    if stage.verification.success_artifact == stage.verification.result_artifact {
        return Err(PromptSequenceError::Invalid(
            "verification success and result artifacts must be distinct".to_owned(),
        ));
    }
    if let Some(log) = &stage.verification.log_artifact {
        validate_safe_name("verification.log_artifact", log)?;
    }
    validate_identity("stage.context_policy_ref", &stage.context_policy_ref, 128)?;
    if stage.outputs.len() > MAX_DECLARED_OUTPUTS {
        return Err(PromptSequenceError::Bounds {
            location: format!("{location}.outputs"),
            reason: format!("at most {MAX_DECLARED_OUTPUTS} outputs are allowed"),
        });
    }
    let mut outputs = BTreeSet::new();
    for output in &stage.outputs {
        validate_safe_name("declared output", &output.name)?;
        validate_text("declared output media_type", &output.media_type, 1, 255)?;
        if !outputs.insert(&output.name) {
            return Err(PromptSequenceError::Invalid(format!(
                "duplicate declared output '{}'",
                output.name
            )));
        }
    }
    Ok(())
}

fn validate_profile(profile: &CapabilityProfileRef) -> Result<(), PromptSequenceError> {
    if profile.operation.as_str() != "process.execute"
        || profile.provider_profile.is_some()
        || profile.execution_trust != ExecutionTrustClass::TrustedHostProcess
    {
        return Err(PromptSequenceError::Invalid(
            "schema-v2 sequence profiles must select process.execute without a provider_profile and require trusted_host_process execution"
                .to_owned(),
        ));
    }
    Ok(())
}

fn validate_repository(profile: &RepositoryWorkspaceProfile) -> Result<(), PromptSequenceError> {
    validate_identity("repository.id", &profile.id, 128)?;
    validate_identity("repository.root_ref", &profile.root_ref, 192)?;
    if profile.allowed_paths.is_empty() || profile.allowed_paths.len() > MAX_ALLOWED_PATHS {
        return Err(PromptSequenceError::Bounds {
            location: "repository.allowed_paths".to_owned(),
            reason: format!("must contain 1..={MAX_ALLOWED_PATHS} relative paths"),
        });
    }
    for path in &profile.allowed_paths {
        validate_relative_path(path)?;
    }
    if !profile
        .allowed_operations
        .contains(&RepositoryOperation::Read)
        || !profile
            .allowed_operations
            .contains(&RepositoryOperation::Execute)
        || !profile
            .allowed_operations
            .contains(&RepositoryOperation::Write)
    {
        return Err(PromptSequenceError::Invalid(
            "repository operations must explicitly include read, write, and execute".to_owned(),
        ));
    }
    if profile.isolation == RepositoryIsolation::IsolatedWorktrees
        && !profile
            .allowed_operations
            .contains(&RepositoryOperation::VersionControl)
    {
        return Err(PromptSequenceError::Invalid(
            "isolated_worktrees requires the version_control operation class".to_owned(),
        ));
    }
    if !profile.artifacts.require_starting_state
        || !profile.artifacts.require_diff
        || !profile.artifacts.require_verification_evidence
    {
        return Err(PromptSequenceError::Invalid(
            "schema-v2 repository artifact policy must require starting state, diff, and verification evidence"
                .to_owned(),
        ));
    }
    for reference in profile
        .credential_refs
        .iter()
        .chain(&profile.remote_access_refs)
    {
        validate_identity("repository reference", reference, 192)?;
    }
    if profile.credential_refs.len() > 32 || profile.remote_access_refs.len() > 32 {
        return Err(PromptSequenceError::Bounds {
            location: "repository.references".to_owned(),
            reason: "credential and remote reference lists are limited to 32 items".to_owned(),
        });
    }
    if profile
        .starting_revision
        .as_ref()
        .is_some_and(|value| value.is_empty() || value.len() > 256 || value.contains('\0'))
    {
        return Err(PromptSequenceError::Invalid(
            "repository starting revision violates its bound".to_owned(),
        ));
    }
    Ok(())
}

fn validate_sequence_budget(budget: PromptSequenceBudget) -> Result<(), PromptSequenceError> {
    if budget.max_review_loops == 0 {
        return Err(PromptSequenceError::Invalid(
            "sequence budget max_review_loops must be nonzero".to_owned(),
        ));
    }
    Ok(())
}

fn validate_extensions(extensions: &BTreeMap<String, Value>) -> Result<(), PromptSequenceError> {
    if extensions.len() > MAX_EXTENSIONS {
        return Err(PromptSequenceError::Bounds {
            location: "sequence.extensions".to_owned(),
            reason: format!("at most {MAX_EXTENSIONS} extensions are allowed"),
        });
    }
    for key in extensions.keys() {
        if key.len() > 192
            || !key.is_ascii()
            || !key.split_once('/').is_some_and(|(namespace, name)| {
                namespace.contains('.') && !namespace.is_empty() && !name.is_empty()
            })
        {
            return Err(PromptSequenceError::Invalid(
                "extension keys must use a DNS namespace and slash".to_owned(),
            ));
        }
    }
    Ok(())
}

fn validate_relative_path(path: &str) -> Result<(), PromptSequenceError> {
    if path.is_empty()
        || path.len() > 1_024
        || path.contains('\0')
        || path.contains('\\')
        || path.starts_with('/')
        || path.as_bytes().get(1) == Some(&b':')
        || path
            .split('/')
            .any(|part| part.is_empty() || matches!(part, "." | ".."))
    {
        return Err(PromptSequenceError::Invalid(
            "repository allowed paths must be bounded relative normal paths".to_owned(),
        ));
    }
    Ok(())
}

fn validate_safe_name(location: &str, value: &str) -> Result<(), PromptSequenceError> {
    validate_identity(location, value, 96)
}

fn validate_namespaced_data_id(
    location: &str,
    value: &str,
    max: usize,
) -> Result<(), PromptSequenceError> {
    validate_identity(location, value, max)?;
    if !value.contains('.') {
        return Err(PromptSequenceError::Invalid(format!(
            "{location} must be namespaced"
        )));
    }
    Ok(())
}

fn validate_identity(location: &str, value: &str, max: usize) -> Result<(), PromptSequenceError> {
    if value.is_empty()
        || value.len() > max
        || !value.is_ascii()
        || !value.as_bytes()[0].is_ascii_alphanumeric()
        || !value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':' | b'/')
        })
    {
        return Err(PromptSequenceError::Invalid(format!(
            "{location} must contain 1..={max} safe ASCII identity bytes"
        )));
    }
    Ok(())
}

fn validate_text(
    location: &str,
    value: &str,
    min: usize,
    max: usize,
) -> Result<(), PromptSequenceError> {
    if value.len() < min || value.len() > max || value.contains('\0') {
        return Err(PromptSequenceError::Bounds {
            location: location.to_owned(),
            reason: format!("text must contain {min}..={max} NUL-free bytes"),
        });
    }
    Ok(())
}

fn map_bound(bound: milkdrift_contracts::JsonBoundViolation) -> PromptSequenceError {
    let kind = match bound.kind() {
        JsonBoundKind::Depth => "depth",
        JsonBoundKind::String => "string bytes",
        JsonBoundKind::Key => "key bytes",
        JsonBoundKind::Array => "array items",
        JsonBoundKind::Object => "object entries",
    };
    PromptSequenceError::Bounds {
        location: bound.path().to_owned(),
        reason: format!("{kind} exceed {}", bound.maximum()),
    }
}
