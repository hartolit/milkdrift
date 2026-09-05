use serde::{Deserialize, Serialize};

use milkdrift_blueprint::{ContentDigest as PolicyDigest, ContextSemanticRole, NodeId, RevisionId};
use milkdrift_capability::InvocationValueReference;
use milkdrift_persistence::{AttemptId, EventId, NodeExecutionId, RunSequence};
use milkdrift_workspace::{
    ArtifactReference, ArtifactSensitivity, CausalReference, ContentDigest, RunId, ScopeReference,
    WorkspaceValueReference,
};

use crate::{ModelContractError, document::encode};

/// Current context-manifest schema with materialization digests and exact producer provenance.
const CONTEXT_MANIFEST_SCHEMA_VERSION_V2: u32 = 2;
const MAX_ENTRIES: usize = 4_096;
const MAX_OMISSIONS: usize = 4_096;
const MAX_EVIDENCE: usize = 256;

/// Lowercase digest of one canonical context manifest.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(transparent)]
pub struct ContextManifestDigest(String);

impl ContextManifestDigest {
    /// Validates a domain-separated digest string.
    pub fn new(value: impl Into<String>) -> Result<Self, ModelContractError> {
        let value = value.into();
        if !milkdrift_contracts::is_canonical_blake3_digest(&value) {
            return Err(ModelContractError::Invalid(
                "context manifest digest must be b3_ plus 64 lowercase hexadecimal characters"
                    .to_owned(),
            ));
        }
        Ok(Self(value))
    }

    /// Digest text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Exact immutable reference persisted and bound to an invocation.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ContextManifestReference {
    /// Manifest contract version.
    pub schema_version: u32,
    /// Canonical manifest digest.
    pub digest: ContextManifestDigest,
    /// Immutable artifact containing the exact canonical manifest.
    pub artifact: ArtifactReference,
}

/// Stable semantic category of one selected item.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ContextSemanticKind {
    /// Direct current-task input.
    DirectInput,
    /// Successful prior output.
    SuccessfulOutput,
    /// Failure or uncertainty evidence.
    Failure,
    /// Decision, approval, or reconciliation fact.
    Decision,
    /// Artifact selected by metadata.
    Artifact,
    /// Raw bounded progress.
    RawProgress,
    /// Tool trace.
    ToolTrace,
    /// Verbose command output.
    VerboseCommandOutput,
    /// Prior prompt.
    PriorPrompt,
    /// Prior final output.
    FinalOutput,
}

/// Exact immutable source of one manifest entry.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "snake_case", tag = "type", deny_unknown_fields)]
pub enum ContextSource {
    /// Named request input.
    DirectInput {
        /// Stable input name.
        name: String,
        /// Exact supplied input reference.
        reference: InvocationValueReference,
    },
    /// Exact node execution and optional journal event.
    NodeExecution {
        /// Semantic source node under the governing revision.
        node: NodeId,
        /// Exact execution.
        execution: NodeExecutionId,
        /// Exact attempt, if the evidence was attempt-specific.
        attempt: Option<AttemptId>,
        /// Exact source journal sequence when compacted detail is involved.
        event_sequence: Option<RunSequence>,
    },
    /// Exact immutable journal event.
    Event {
        /// Event identity.
        event: EventId,
        /// Aggregate sequence.
        sequence: RunSequence,
    },
    /// Exact immutable workspace value.
    WorkspaceValue {
        /// Exact immutable workspace value.
        reference: WorkspaceValueReference,
    },
    /// Exact immutable artifact reference.
    Artifact {
        /// Exact immutable artifact.
        reference: ArtifactReference,
    },
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case", tag = "type", deny_unknown_fields)]
enum ContextSourceWire {
    DirectInput {
        name: String,
        reference: InvocationValueReference,
    },
    NodeExecution {
        node: NodeId,
        execution: NodeExecutionId,
        attempt: Option<AttemptId>,
        event_sequence: Option<RunSequence>,
    },
    Event {
        event: EventId,
        sequence: RunSequence,
    },
    WorkspaceValue {
        reference: WorkspaceValueReference,
    },
    Artifact {
        reference: ArtifactReference,
    },
}

milkdrift_contracts::deserialize_via!(ContextSource, ContextSourceWire, |wire| {
    let source = match wire {
        ContextSourceWire::DirectInput { name, reference } => Self::DirectInput { name, reference },
        ContextSourceWire::NodeExecution {
            node,
            execution,
            attempt,
            event_sequence,
        } => Self::NodeExecution {
            node,
            execution,
            attempt,
            event_sequence,
        },
        ContextSourceWire::Event { event, sequence } => Self::Event { event, sequence },
        ContextSourceWire::WorkspaceValue { reference } => Self::WorkspaceValue { reference },
        ContextSourceWire::Artifact { reference } => Self::Artifact { reference },
    };
    source.validate().map(|()| source)
});

impl ContextSource {
    fn validate(&self) -> Result<(), ModelContractError> {
        if let Self::DirectInput { name, .. } = self
            && (name.is_empty() || name.len() > 128 || !name.is_ascii())
        {
            return Err(ModelContractError::Invalid(
                "context input source name must contain 1..=128 ASCII bytes".to_owned(),
            ));
        }
        Ok(())
    }
}

/// Small causal/evidence reference attached to an entry.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case", tag = "type", deny_unknown_fields)]
pub enum ContextEvidenceReference {
    /// Existing workspace causal reference.
    Workspace {
        /// Existing workspace provenance fact.
        reference: CausalReference,
    },
    /// Exact prior execution.
    Execution {
        /// Exact prior execution.
        execution: NodeExecutionId,
    },
    /// Exact journal event.
    Event {
        /// Exact event identity.
        event: EventId,
        /// Exact aggregate sequence.
        sequence: RunSequence,
    },
}

/// Authority facts recorded without embedding grants, tokens, or secret values.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AuthorityFact {
    /// Whether the source required explicit authority.
    pub required: bool,
    /// Whether exact authority was proven during selection.
    pub authorized: bool,
    /// Bounded non-secret decision/grant identity, if applicable.
    pub authority_reference: Option<String>,
}

/// Exact bounded producer facts known when evidence was created.
#[derive(Clone, Debug, Default, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ContextProducerFact {
    /// Actor/controller identity when recorded by the typed source fact.
    pub actor: Option<String>,
    /// Exact capability generation identity when executor-backed.
    pub capability: Option<String>,
    /// Exact descriptor generation when executor-backed.
    pub descriptor_revision: Option<u64>,
    /// Exact provider profile, when one governed production.
    pub provider_profile: Option<String>,
    /// Authenticated peer identity, when production was remote.
    pub peer: Option<String>,
    /// Exact provider-neutral invocation identity.
    pub invocation: Option<String>,
}

impl ContextProducerFact {
    fn validate(&self) -> Result<(), ModelContractError> {
        if self.descriptor_revision == Some(0)
            || [
                &self.actor,
                &self.capability,
                &self.provider_profile,
                &self.peer,
                &self.invocation,
            ]
            .into_iter()
            .flatten()
            .any(|value| value.is_empty() || value.len() > 192 || !value.is_ascii())
        {
            return Err(ModelContractError::Invalid(
                "invalid bounded context producer provenance".to_owned(),
            ));
        }
        Ok(())
    }
}

/// Why an entry was included.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ContextInclusionReason {
    /// Direct task input policy.
    DirectInput,
    /// Explicit incoming graph ancestry.
    CausalAncestor,
    /// Exact node selector.
    SelectedNode,
    /// Known semantic-role selector.
    SelectedRole,
    /// Category selector.
    IncludedCategory,
    /// Artifact metadata selector.
    ArtifactSelector,
    /// Explicit continuation reference.
    Continuation,
    /// Exact durable evidence reference declared by workflow or operator policy.
    ExplicitEvidence,
}

/// Stable omission/truncation reason.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ContextOmissionReason {
    /// Candidate did not match a safe declarative selector.
    NotSelected,
    /// Category was explicitly excluded.
    ExcludedCategory,
    /// Candidate belongs to a sibling branch not exposed by a join/edge.
    BranchIsolated,
    /// Required authority was absent.
    AuthorityDenied,
    /// Referenced evidence was missing or contradicted its immutable integrity facts.
    MissingOrCorrupt,
    /// The source or provider cannot represent this evidence safely.
    Unsupported,
    /// An exact newer durable fact superseded this optional candidate.
    Superseded,
    /// Item count budget would be crossed.
    ItemBudget,
    /// Inline/reference-byte budget would be crossed.
    ByteBudget,
    /// Artifact byte budget would be crossed.
    ArtifactByteBudget,
    /// Optional model-input-unit budget would be crossed.
    ModelInputUnitBudget,
    /// One item exceeded its materialization byte ceiling.
    PerItemByteBudget,
    /// Selected artifact count would be crossed.
    ArtifactItemBudget,
    /// Selection stopped after an earlier deterministic overflow.
    SelectionStopped,
}

/// One exact selected item. It contains references and small facts, never secret values.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ContextManifestEntry {
    ordinal: u32,
    kind: ContextSemanticKind,
    semantic_roles: std::collections::BTreeSet<ContextSemanticRole>,
    source: ContextSource,
    content_digest: ContentDigest,
    source_revision: RevisionId,
    source_execution: Option<NodeExecutionId>,
    source_attempt: Option<AttemptId>,
    source_scope: Option<ScopeReference>,
    causal_distance: Option<u16>,
    source_sequence: Option<RunSequence>,
    occurred_at_ms: Option<u64>,
    producer: ContextProducerFact,
    causal_parents: Vec<ContextEvidenceReference>,
    selected_artifact: bool,
    selected_bytes: u64,
    selected_artifact_bytes: u64,
    estimated_model_input_units: Option<u64>,
    sensitivity: ArtifactSensitivity,
    authority: AuthorityFact,
    reason: ContextInclusionReason,
}

impl ContextManifestEntry {
    /// Constructs a validated selected entry.
    #[allow(clippy::too_many_arguments)] // One validated model contract keeps its exact context or task facts explicit.
    pub fn new(
        ordinal: u32,
        kind: ContextSemanticKind,
        semantic_roles: std::collections::BTreeSet<ContextSemanticRole>,
        source: ContextSource,
        content_digest: ContentDigest,
        source_revision: RevisionId,
        source_execution: Option<NodeExecutionId>,
        source_attempt: Option<AttemptId>,
        source_scope: Option<ScopeReference>,
        causal_distance: Option<u16>,
        source_sequence: Option<RunSequence>,
        occurred_at_ms: Option<u64>,
        producer: ContextProducerFact,
        causal_parents: Vec<ContextEvidenceReference>,
        selected_artifact: bool,
        selected_bytes: u64,
        selected_artifact_bytes: u64,
        estimated_model_input_units: Option<u64>,
        sensitivity: ArtifactSensitivity,
        authority: AuthorityFact,
        reason: ContextInclusionReason,
    ) -> Result<Self, ModelContractError> {
        source.validate()?;
        producer.validate()?;
        let source_consistent = match &source {
            ContextSource::Artifact { reference } => {
                selected_artifact
                    && reference.digest() == content_digest
                    && reference.size_bytes() == selected_artifact_bytes
            }
            ContextSource::DirectInput {
                reference: InvocationValueReference::Artifact { reference },
                ..
            } => {
                selected_artifact
                    && reference.digest() == content_digest.to_hex()
                    && reference.size_bytes() == Some(selected_artifact_bytes)
            }
            ContextSource::NodeExecution {
                execution, attempt, ..
            } => {
                source_execution.as_ref() == Some(execution)
                    && source_attempt.as_ref() == attempt.as_ref()
            }
            ContextSource::DirectInput { .. }
            | ContextSource::Event { .. }
            | ContextSource::WorkspaceValue { .. } => true,
        };
        if ordinal == 0
            || semantic_roles.len() > MAX_EVIDENCE
            || causal_parents.len() > MAX_EVIDENCE
            || !source_consistent
            || source_attempt.is_some() && source_execution.is_none()
            || !selected_artifact && selected_artifact_bytes != 0
            || !selected_artifact
                && matches!(
                    &source,
                    ContextSource::Artifact { .. }
                        | ContextSource::DirectInput {
                            reference: InvocationValueReference::Artifact { .. },
                            ..
                        }
                )
            || authority.required && !authority.authorized
            || authority
                .authority_reference
                .as_ref()
                .is_some_and(|value| value.is_empty() || value.len() > 192)
        {
            return Err(ModelContractError::Invalid(
                "invalid selected context entry facts".to_owned(),
            ));
        }
        Ok(Self {
            ordinal,
            kind,
            semantic_roles,
            source,
            content_digest,
            source_revision,
            source_execution,
            source_attempt,
            source_scope,
            causal_distance,
            source_sequence,
            occurred_at_ms,
            producer,
            causal_parents,
            selected_artifact,
            selected_bytes,
            selected_artifact_bytes,
            estimated_model_input_units,
            sensitivity,
            authority,
            reason,
        })
    }
    /// Stable one-based selection ordinal.
    #[must_use]
    pub const fn ordinal(&self) -> u32 {
        self.ordinal
    }
    /// Semantic kind.
    #[must_use]
    pub const fn kind(&self) -> ContextSemanticKind {
        self.kind
    }
    /// Semantic roles published by the source task or assigned to this durable fact.
    #[must_use]
    pub const fn semantic_roles(&self) -> &std::collections::BTreeSet<ContextSemanticRole> {
        &self.semantic_roles
    }
    /// Exact source.
    #[must_use]
    pub const fn source(&self) -> &ContextSource {
        &self.source
    }
    /// Digest of the exact bytes materialized for this item.
    #[must_use]
    pub const fn content_digest(&self) -> ContentDigest {
        self.content_digest
    }
    /// Revision governing the source occurrence.
    #[must_use]
    pub const fn source_revision(&self) -> &RevisionId {
        &self.source_revision
    }
    /// Exact source execution when execution-backed.
    #[must_use]
    pub const fn source_execution(&self) -> Option<&NodeExecutionId> {
        self.source_execution.as_ref()
    }
    /// Exact source attempt when the evidence is attempt-specific.
    #[must_use]
    pub const fn source_attempt(&self) -> Option<&AttemptId> {
        self.source_attempt.as_ref()
    }
    /// Exact source workspace scope.
    #[must_use]
    pub const fn source_scope(&self) -> Option<&ScopeReference> {
        self.source_scope.as_ref()
    }
    /// Frozen causal graph distance under the source occurrence's revision.
    #[must_use]
    pub const fn causal_distance(&self) -> Option<u16> {
        self.causal_distance
    }
    /// Exact source journal sequence.
    #[must_use]
    pub const fn source_sequence(&self) -> Option<RunSequence> {
        self.source_sequence
    }
    /// Source timestamp when semantically relevant.
    #[must_use]
    pub const fn occurred_at_ms(&self) -> Option<u64> {
        self.occurred_at_ms
    }
    /// Exact bounded producer provenance.
    #[must_use]
    pub const fn producer(&self) -> &ContextProducerFact {
        &self.producer
    }
    /// Causal/evidence references.
    #[must_use]
    pub fn causal_parents(&self) -> &[ContextEvidenceReference] {
        &self.causal_parents
    }
    /// Whether this selection materializes an artifact, including a zero-byte artifact.
    #[must_use]
    pub const fn selected_artifact(&self) -> bool {
        self.selected_artifact
    }
    /// Selected small/reference bytes.
    #[must_use]
    pub const fn selected_bytes(&self) -> u64 {
        self.selected_bytes
    }
    /// Selected artifact bytes.
    #[must_use]
    pub const fn selected_artifact_bytes(&self) -> u64 {
        self.selected_artifact_bytes
    }
    /// Optional model-unit estimate.
    #[must_use]
    pub const fn estimated_model_input_units(&self) -> Option<u64> {
        self.estimated_model_input_units
    }
    /// Sensitivity propagated to outputs.
    #[must_use]
    pub const fn sensitivity(&self) -> ArtifactSensitivity {
        self.sensitivity
    }
    /// Authority facts.
    #[must_use]
    pub const fn authority(&self) -> &AuthorityFact {
        &self.authority
    }
    /// Stable inclusion reason.
    #[must_use]
    pub const fn reason(&self) -> ContextInclusionReason {
        self.reason
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ContextManifestEntryWire {
    ordinal: u32,
    kind: ContextSemanticKind,
    semantic_roles: std::collections::BTreeSet<ContextSemanticRole>,
    source: ContextSource,
    content_digest: ContentDigest,
    source_revision: RevisionId,
    source_execution: Option<NodeExecutionId>,
    source_attempt: Option<AttemptId>,
    source_scope: Option<ScopeReference>,
    causal_distance: Option<u16>,
    source_sequence: Option<RunSequence>,
    occurred_at_ms: Option<u64>,
    producer: ContextProducerFact,
    causal_parents: Vec<ContextEvidenceReference>,
    selected_artifact: bool,
    selected_bytes: u64,
    selected_artifact_bytes: u64,
    estimated_model_input_units: Option<u64>,
    sensitivity: ArtifactSensitivity,
    authority: AuthorityFact,
    reason: ContextInclusionReason,
}

milkdrift_contracts::deserialize_via!(ContextManifestEntry, ContextManifestEntryWire, |w| {
    Self::new(
        w.ordinal,
        w.kind,
        w.semantic_roles,
        w.source,
        w.content_digest,
        w.source_revision,
        w.source_execution,
        w.source_attempt,
        w.source_scope,
        w.causal_distance,
        w.source_sequence,
        w.occurred_at_ms,
        w.producer,
        w.causal_parents,
        w.selected_artifact,
        w.selected_bytes,
        w.selected_artifact_bytes,
        w.estimated_model_input_units,
        w.sensitivity,
        w.authority,
        w.reason,
    )
});

/// One non-selected candidate summary, kept bounded and deterministic.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ContextOmission {
    /// Exact source when it could be resolved.
    pub source: Option<ContextSource>,
    /// Semantic category.
    pub kind: ContextSemanticKind,
    /// Stable omission reason.
    pub reason: ContextOmissionReason,
    /// Whether this candidate was required.
    pub required: bool,
    /// Candidate bytes not selected.
    pub omitted_bytes: u64,
    /// Candidate artifact bytes not selected.
    pub omitted_artifact_bytes: u64,
}

/// Exact totals checked incrementally by the builder.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ContextTotals {
    /// Entry count.
    pub items: u32,
    /// Selected small/reference bytes.
    pub bytes: u64,
    /// Selected artifact bytes.
    pub artifact_bytes: u64,
    /// Selected artifact-reference count.
    pub artifacts: u32,
    /// Sum of supplied model-unit estimates; absent if no estimate was supplied.
    pub model_input_units: Option<u64>,
}

/// Exact immutable selection used for one invocation attempt.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ContextManifest {
    schema_version: u32,
    run: RunId,
    revision: RevisionId,
    node: NodeId,
    execution: NodeExecutionId,
    attempt: AttemptId,
    policy_version: u32,
    policy_digest: PolicyDigest,
    entries: Vec<ContextManifestEntry>,
    omissions: Vec<ContextOmission>,
    totals: ContextTotals,
    budget: milkdrift_blueprint::ContextBudget,
    digest: ContextManifestDigest,
}

#[derive(Serialize)]
struct DigestInput<'a> {
    schema_version: u32,
    run: &'a RunId,
    revision: &'a RevisionId,
    node: &'a NodeId,
    execution: &'a NodeExecutionId,
    attempt: &'a AttemptId,
    policy_version: u32,
    policy_digest: &'a PolicyDigest,
    entries: &'a [ContextManifestEntry],
    omissions: &'a [ContextOmission],
    totals: ContextTotals,
    budget: milkdrift_blueprint::ContextBudget,
}

impl ContextManifest {
    /// Validates exact totals/order and computes a deterministic manifest digest.
    #[allow(clippy::too_many_arguments)] // One validated model contract keeps its exact context or task facts explicit.
    pub fn new(
        run: RunId,
        revision: RevisionId,
        node: NodeId,
        execution: NodeExecutionId,
        attempt: AttemptId,
        policy_version: u32,
        policy_digest: PolicyDigest,
        entries: Vec<ContextManifestEntry>,
        omissions: Vec<ContextOmission>,
        totals: ContextTotals,
        budget: milkdrift_blueprint::ContextBudget,
    ) -> Result<Self, ModelContractError> {
        if policy_version == 0 || entries.len() > MAX_ENTRIES || omissions.len() > MAX_OMISSIONS {
            return Err(ModelContractError::Invalid(
                "context manifest count/version bound exceeded".to_owned(),
            ));
        }
        for (index, entry) in entries.iter().enumerate() {
            let expected = u32::try_from(index)
                .ok()
                .and_then(|v| v.checked_add(1))
                .ok_or_else(|| {
                    ModelContractError::Invalid("context ordinal overflow".to_owned())
                })?;
            if entry.ordinal() != expected {
                return Err(ModelContractError::Invalid(
                    "context entry ordinals must be contiguous from one".to_owned(),
                ));
            }
        }
        let actual = totals_for(&entries)?;
        if actual != totals
            || totals.items > budget.max_items
            || totals.bytes > budget.max_bytes
            || totals.artifact_bytes > budget.max_artifact_bytes
            || totals.artifacts > budget.max_artifacts
            || budget
                .max_model_input_units
                .zip(totals.model_input_units)
                .is_some_and(|(max, used)| used > max)
        {
            return Err(ModelContractError::Invalid(
                "context totals contradict entries or budget".to_owned(),
            ));
        }
        let input = DigestInput {
            schema_version: CONTEXT_MANIFEST_SCHEMA_VERSION_V2,
            run: &run,
            revision: &revision,
            node: &node,
            execution: &execution,
            attempt: &attempt,
            policy_version,
            policy_digest: &policy_digest,
            entries: &entries,
            omissions: &omissions,
            totals,
            budget,
        };
        let bytes = encode(&input)?;
        let mut hasher = blake3::Hasher::new();
        hasher.update(b"milkdrift.context-manifest.v2\0");
        hasher.update(&bytes);
        let digest = ContextManifestDigest::new(format!("b3_{}", hasher.finalize()))?;
        Ok(Self {
            schema_version: CONTEXT_MANIFEST_SCHEMA_VERSION_V2,
            run,
            revision,
            node,
            execution,
            attempt,
            policy_version,
            policy_digest,
            entries,
            omissions,
            totals,
            budget,
            digest,
        })
    }
    /// Owning run.
    #[must_use]
    pub const fn run(&self) -> &RunId {
        &self.run
    }
    /// Exact governing revision.
    #[must_use]
    pub const fn revision(&self) -> &RevisionId {
        &self.revision
    }
    /// Current semantic node.
    #[must_use]
    pub const fn node(&self) -> &NodeId {
        &self.node
    }
    /// Current execution.
    #[must_use]
    pub const fn execution(&self) -> &NodeExecutionId {
        &self.execution
    }
    /// Frozen attempt.
    #[must_use]
    pub const fn attempt(&self) -> &AttemptId {
        &self.attempt
    }
    /// Context policy schema.
    #[must_use]
    pub const fn policy_version(&self) -> u32 {
        self.policy_version
    }
    /// Exact policy digest.
    #[must_use]
    pub const fn policy_digest(&self) -> &PolicyDigest {
        &self.policy_digest
    }
    /// Ordered selected entries.
    #[must_use]
    pub fn entries(&self) -> &[ContextManifestEntry] {
        &self.entries
    }
    /// Ordered omissions.
    #[must_use]
    pub fn omissions(&self) -> &[ContextOmission] {
        &self.omissions
    }
    /// Exact totals.
    #[must_use]
    pub const fn totals(&self) -> ContextTotals {
        self.totals
    }
    /// Exact applied budget.
    #[must_use]
    pub const fn budget(&self) -> milkdrift_blueprint::ContextBudget {
        self.budget
    }
    /// Canonical digest.
    #[must_use]
    pub const fn digest(&self) -> &ContextManifestDigest {
        &self.digest
    }
    /// Exact manifest schema version.
    #[must_use]
    pub const fn schema_version(&self) -> u32 {
        self.schema_version
    }

    /// Rebinds the exact frozen selection to a deliberate new attempt without consulting history.
    pub fn rebind_attempt(&self, attempt: AttemptId) -> Result<Self, ModelContractError> {
        Self::new(
            self.run.clone(),
            self.revision.clone(),
            self.node.clone(),
            self.execution.clone(),
            attempt,
            self.policy_version,
            self.policy_digest.clone(),
            self.entries.clone(),
            self.omissions.clone(),
            self.totals,
            self.budget,
        )
    }
}

impl<'de> Deserialize<'de> for ContextManifest {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Wire {
            schema_version: u32,
            run: RunId,
            revision: RevisionId,
            node: NodeId,
            execution: NodeExecutionId,
            attempt: AttemptId,
            policy_version: u32,
            policy_digest: PolicyDigest,
            entries: Vec<ContextManifestEntry>,
            omissions: Vec<ContextOmission>,
            totals: ContextTotals,
            budget: milkdrift_blueprint::ContextBudget,
            digest: ContextManifestDigest,
        }
        let w = Wire::deserialize(deserializer)?;
        if w.schema_version != CONTEXT_MANIFEST_SCHEMA_VERSION_V2 {
            return Err(serde::de::Error::custom(
                "unsupported context manifest version",
            ));
        }
        let expected = w.digest.clone();
        let value = Self::new(
            w.run,
            w.revision,
            w.node,
            w.execution,
            w.attempt,
            w.policy_version,
            w.policy_digest,
            w.entries,
            w.omissions,
            w.totals,
            w.budget,
        )
        .map_err(serde::de::Error::custom)?;
        if value.digest != expected {
            return Err(serde::de::Error::custom("context manifest digest mismatch"));
        }
        Ok(value)
    }
}

fn totals_for(entries: &[ContextManifestEntry]) -> Result<ContextTotals, ModelContractError> {
    let mut totals = ContextTotals::default();
    let mut any_units = false;
    let mut units = 0u64;
    for entry in entries {
        totals.items = totals
            .items
            .checked_add(1)
            .ok_or_else(|| ModelContractError::Invalid("item total overflow".to_owned()))?;
        totals.bytes = totals
            .bytes
            .checked_add(entry.selected_bytes())
            .ok_or_else(|| ModelContractError::Invalid("byte total overflow".to_owned()))?;
        totals.artifact_bytes = totals
            .artifact_bytes
            .checked_add(entry.selected_artifact_bytes())
            .ok_or_else(|| ModelContractError::Invalid("artifact total overflow".to_owned()))?;
        if entry.selected_artifact() {
            totals.artifacts = totals.artifacts.checked_add(1).ok_or_else(|| {
                ModelContractError::Invalid("artifact item total overflow".to_owned())
            })?;
        }
        if let Some(value) = entry.estimated_model_input_units() {
            any_units = true;
            units = units.checked_add(value).ok_or_else(|| {
                ModelContractError::Invalid("model unit total overflow".to_owned())
            })?;
        }
    }
    totals.model_input_units = any_units.then_some(units);
    Ok(totals)
}
