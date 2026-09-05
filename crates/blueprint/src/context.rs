use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use milkdrift_contracts::{JsonLimits, canonical_json_bytes};

use crate::{ContentDigest, ModelError, NodeId};

const MAX_SELECTORS: usize = 256;
const MAX_SELECTOR_TEXT_BYTES: usize = 255;
const MAX_EXACT_SOURCE_BYTES: usize = 1_024;
const POLICY_JSON_LIMITS: JsonLimits = JsonLimits {
    maximum_depth: 16,
    maximum_string_bytes: MAX_EXACT_SOURCE_BYTES,
    maximum_key_bytes: 128,
    maximum_container_items: MAX_SELECTORS,
};

/// Stable semantic role understood by causal context selection.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ContextSemanticRole {
    /// Governing instruction established by the workflow author.
    SystemInstruction,
    /// Developer-authored instruction beneath the system policy.
    DeveloperInstruction,
    /// User-supplied request or correction.
    UserRequest,
    /// Evidence produced by a tool or prior task.
    Evidence,
    /// Explicit decision, approval, or reconciliation fact.
    Decision,
    /// Failure or uncertainty evidence.
    FailureEvidence,
    /// Product or workflow requirement.
    Requirement,
    /// Implementation or source change.
    Implementation,
    /// Verification result, successful or failed.
    Verification,
    /// Reviewer or controller result.
    Review,
}

/// Context categories that can be explicitly included or excluded.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ContextCategory {
    /// Direct inputs declared for the current task.
    DirectInput,
    /// Successful task output.
    SuccessfulOutput,
    /// Failure and uncertainty evidence.
    Failure,
    /// Approval, decision, or reconciliation fact.
    Decision,
    /// Artifact metadata or bytes.
    Artifact,
    /// Bounded raw progress observations.
    RawProgress,
    /// Tool request/response traces.
    ToolTrace,
    /// Verbose command output.
    VerboseCommandOutput,
    /// Prior model prompts.
    PriorPrompt,
    /// Prior final model/task output.
    FinalOutput,
}

/// Artifact sensitivity selector kept semantic and independent from storage mechanics.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ContextArtifactSensitivity {
    /// Public artifact.
    Public,
    /// Internal artifact.
    Internal,
    /// Restricted artifact requiring explicit read authority.
    Restricted,
}

/// Artifact retention class selector.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ContextArtifactRetention {
    /// Content remains while referenced.
    WhileReferenced,
    /// Content has a bounded minimum deadline.
    Until,
    /// Content is retained indefinitely.
    Indefinite,
}

/// Stable provenance class used by declarative artifact filters.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ContextProvenanceClass {
    /// Declared run input.
    RunInput,
    /// Exact workspace value.
    WorkspaceValue,
    /// Exact artifact.
    Artifact,
    /// Exact capability invocation.
    Invocation,
    /// Bounded external source reference.
    External,
}

/// Declarative artifact metadata filter. Empty sets are wildcards.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ContextArtifactSelector {
    names: BTreeSet<String>,
    media_types: BTreeSet<String>,
    sensitivities: BTreeSet<ContextArtifactSensitivity>,
    retentions: BTreeSet<ContextArtifactRetention>,
    provenance: BTreeSet<ContextProvenanceClass>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ContextArtifactSelectorWire {
    names: BTreeSet<String>,
    media_types: BTreeSet<String>,
    sensitivities: BTreeSet<ContextArtifactSensitivity>,
    retentions: BTreeSet<ContextArtifactRetention>,
    provenance: BTreeSet<ContextProvenanceClass>,
}

milkdrift_contracts::deserialize_via!(
    ContextArtifactSelector,
    ContextArtifactSelectorWire,
    |wire| Self::new(
        wire.names,
        wire.media_types,
        wire.sensitivities,
        wire.retentions,
        wire.provenance,
    )
);

impl ContextArtifactSelector {
    /// Creates and bounds an artifact metadata selector.
    pub fn new(
        names: BTreeSet<String>,
        media_types: BTreeSet<String>,
        sensitivities: BTreeSet<ContextArtifactSensitivity>,
        retentions: BTreeSet<ContextArtifactRetention>,
        provenance: BTreeSet<ContextProvenanceClass>,
    ) -> Result<Self, ModelError> {
        validate_text_set("context.artifacts.names", &names)?;
        validate_text_set("context.artifacts.media_types", &media_types)?;
        if sensitivities.len() > MAX_SELECTORS
            || retentions.len() > MAX_SELECTORS
            || provenance.len() > MAX_SELECTORS
        {
            return Err(ModelError::new(
                "context.artifacts",
                "artifact selector cardinality exceeds the supported bound",
            ));
        }
        Ok(Self {
            names,
            media_types,
            sensitivities,
            retentions,
            provenance,
        })
    }

    /// Artifact names; empty accepts every name.
    #[must_use]
    pub const fn names(&self) -> &BTreeSet<String> {
        &self.names
    }

    /// Exact media types; empty accepts every media type.
    #[must_use]
    pub const fn media_types(&self) -> &BTreeSet<String> {
        &self.media_types
    }

    /// Accepted sensitivity classes.
    #[must_use]
    pub const fn sensitivities(&self) -> &BTreeSet<ContextArtifactSensitivity> {
        &self.sensitivities
    }

    /// Accepted retention classes.
    #[must_use]
    pub const fn retentions(&self) -> &BTreeSet<ContextArtifactRetention> {
        &self.retentions
    }

    /// Accepted provenance classes.
    #[must_use]
    pub const fn provenance(&self) -> &BTreeSet<ContextProvenanceClass> {
        &self.provenance
    }
}

/// Hard manifest-selection budgets checked before content is loaded.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ContextBudget {
    /// Maximum selected entries.
    pub max_items: u32,
    /// Maximum selected non-artifact bytes.
    pub max_bytes: u64,
    /// Maximum aggregate referenced artifact bytes.
    pub max_artifact_bytes: u64,
    /// Optional provider-neutral model-input-unit estimate.
    pub max_model_input_units: Option<u64>,
    /// Maximum durable journal records examined while discovering candidates.
    #[serde(
        default = "default_max_candidate_records",
        skip_serializing_if = "is_default_max_candidate_records"
    )]
    pub max_candidate_records: u32,
    /// Maximum selected artifacts, independent of aggregate artifact bytes.
    #[serde(
        default = "default_max_artifacts",
        skip_serializing_if = "is_default_max_artifacts"
    )]
    pub max_artifacts: u32,
    /// Maximum materialized bytes for one selected item.
    #[serde(
        default = "default_max_per_item_bytes",
        skip_serializing_if = "is_default_max_per_item_bytes"
    )]
    pub max_per_item_bytes: u64,
    /// Maximum bounded historical event summaries admitted as candidates.
    #[serde(
        default = "default_max_event_summaries",
        skip_serializing_if = "is_default_max_event_summaries"
    )]
    pub max_event_summaries: u32,
    /// Maximum canonical bytes in the frozen manifest document.
    #[serde(
        default = "default_max_manifest_bytes",
        skip_serializing_if = "is_default_max_manifest_bytes"
    )]
    pub max_manifest_bytes: u64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ContextBudgetWire {
    max_items: u32,
    max_bytes: u64,
    max_artifact_bytes: u64,
    max_model_input_units: Option<u64>,
    #[serde(default = "default_max_candidate_records")]
    max_candidate_records: u32,
    #[serde(default = "default_max_artifacts")]
    max_artifacts: u32,
    #[serde(default = "default_max_per_item_bytes")]
    max_per_item_bytes: u64,
    #[serde(default = "default_max_event_summaries")]
    max_event_summaries: u32,
    #[serde(default = "default_max_manifest_bytes")]
    max_manifest_bytes: u64,
}

milkdrift_contracts::deserialize_via!(ContextBudget, ContextBudgetWire, |wire| {
    let budget = Self {
        max_items: wire.max_items,
        max_bytes: wire.max_bytes,
        max_artifact_bytes: wire.max_artifact_bytes,
        max_model_input_units: wire.max_model_input_units,
        max_candidate_records: wire.max_candidate_records,
        max_artifacts: wire.max_artifacts,
        max_per_item_bytes: wire.max_per_item_bytes,
        max_event_summaries: wire.max_event_summaries,
        max_manifest_bytes: wire.max_manifest_bytes,
    };
    budget.validate().map(|()| budget)
});

impl ContextBudget {
    /// Creates nonzero, defensively bounded budgets.
    pub fn new(
        max_items: u32,
        max_bytes: u64,
        max_artifact_bytes: u64,
        max_model_input_units: Option<u64>,
    ) -> Result<Self, ModelError> {
        if max_items == 0
            || max_items > 65_536
            || max_bytes == 0
            || max_artifact_bytes == 0
            || max_model_input_units == Some(0)
        {
            return Err(ModelError::new(
                "context.budget",
                "budgets must be nonzero and max_items must not exceed 65536",
            ));
        }
        Ok(Self {
            max_items,
            max_bytes,
            max_artifact_bytes,
            max_model_input_units,
            max_artifacts: default_max_artifacts().min(max_items),
            ..Self::default()
        })
    }

    /// Replaces discovery, per-item, artifact-count, event, and manifest bounds.
    pub fn with_discovery_limits(
        mut self,
        max_candidate_records: u32,
        max_artifacts: u32,
        max_per_item_bytes: u64,
        max_event_summaries: u32,
        max_manifest_bytes: u64,
    ) -> Result<Self, ModelError> {
        self.max_candidate_records = max_candidate_records;
        self.max_artifacts = max_artifacts;
        self.max_per_item_bytes = max_per_item_bytes;
        self.max_event_summaries = max_event_summaries;
        self.max_manifest_bytes = max_manifest_bytes;
        self.validate()?;
        Ok(self)
    }

    fn validate(&self) -> Result<(), ModelError> {
        if self.max_items == 0
            || self.max_items > 65_536
            || self.max_bytes == 0
            || self.max_artifact_bytes == 0
            || self.max_model_input_units == Some(0)
            || self.max_candidate_records == 0
            || self.max_candidate_records > 65_536
            || self.max_artifacts == 0
            || self.max_artifacts > self.max_items
            || self.max_per_item_bytes == 0
            || self.max_event_summaries > self.max_candidate_records
            || self.max_manifest_bytes == 0
            || self.max_manifest_bytes > 2_097_152
        {
            return Err(ModelError::new(
                "context.budget",
                "invalid item, byte, discovery, artifact, event-summary, or manifest bound",
            ));
        }
        Ok(())
    }
}

const fn default_max_candidate_records() -> u32 {
    4_096
}

const fn is_default_max_candidate_records(value: &u32) -> bool {
    *value == default_max_candidate_records()
}

const fn default_max_artifacts() -> u32 {
    32
}

const fn is_default_max_artifacts(value: &u32) -> bool {
    *value == default_max_artifacts()
}

const fn default_max_per_item_bytes() -> u64 {
    1_048_576
}

const fn is_default_max_per_item_bytes(value: &u64) -> bool {
    *value == default_max_per_item_bytes()
}

const fn default_max_event_summaries() -> u32 {
    128
}

const fn is_default_max_event_summaries(value: &u32) -> bool {
    *value == default_max_event_summaries()
}

const fn default_max_manifest_bytes() -> u64 {
    524_288
}

const fn is_default_max_manifest_bytes(value: &u64) -> bool {
    *value == default_max_manifest_bytes()
}

impl Default for ContextBudget {
    fn default() -> Self {
        Self {
            max_items: 64,
            max_bytes: 262_144,
            max_artifact_bytes: 16_777_216,
            max_model_input_units: None,
            max_candidate_records: 4_096,
            max_artifacts: 32,
            max_per_item_bytes: 1_048_576,
            max_event_summaries: 128,
            max_manifest_bytes: 524_288,
        }
    }
}

/// Stable ordering applied after causal eligibility is established.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ContextOrdering {
    /// Causal depth, semantic category, source node, execution, then source identity.
    #[default]
    CausalKindSource,
}

/// Deterministic action when an optional candidate crosses a budget.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ContextTruncation {
    /// Omit the candidate and continue evaluating later candidates.
    #[default]
    OmitOversized,
    /// Stop selection at the first candidate that crosses a budget.
    StopAtFirstOverflow,
}

/// Provider session policy selected at blueprint-definition time.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ContextSessionPolicy {
    /// Send the complete frozen manifest without hidden provider state.
    #[default]
    Fresh,
    /// Require an exact continuation reference on each invocation.
    ExplicitContinuation,
    /// Permit an explicitly configured provider-managed session feature.
    ProviderManaged,
}

/// Immutable, declarative context policy owned by a task definition.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TaskContextPolicy {
    include_direct_inputs: bool,
    ancestor_depth: Option<u16>,
    selected_nodes: BTreeSet<NodeId>,
    #[serde(flatten)]
    exact_sources: Box<ExactContextSources>,
    selected_roles: BTreeSet<ContextSemanticRole>,
    include_categories: BTreeSet<ContextCategory>,
    exclude_categories: BTreeSet<ContextCategory>,
    artifact_selector: Option<ContextArtifactSelector>,
    budget: ContextBudget,
    ordering: ContextOrdering,
    truncation: ContextTruncation,
    session: ContextSessionPolicy,
    fail_closed: bool,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
struct ExactContextSources {
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    selected_executions: BTreeSet<String>,
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    selected_workspace_values: BTreeSet<String>,
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    explicit_evidence: BTreeSet<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TaskContextPolicyWire {
    include_direct_inputs: bool,
    ancestor_depth: Option<u16>,
    selected_nodes: BTreeSet<NodeId>,
    #[serde(default)]
    selected_executions: BTreeSet<String>,
    #[serde(default)]
    selected_workspace_values: BTreeSet<String>,
    #[serde(default)]
    explicit_evidence: BTreeSet<String>,
    selected_roles: BTreeSet<ContextSemanticRole>,
    include_categories: BTreeSet<ContextCategory>,
    exclude_categories: BTreeSet<ContextCategory>,
    artifact_selector: Option<ContextArtifactSelector>,
    budget: ContextBudget,
    ordering: ContextOrdering,
    truncation: ContextTruncation,
    session: ContextSessionPolicy,
    fail_closed: bool,
}

milkdrift_contracts::deserialize_via!(TaskContextPolicy, TaskContextPolicyWire, |wire| {
    let policy = Self {
        include_direct_inputs: wire.include_direct_inputs,
        ancestor_depth: wire.ancestor_depth,
        selected_nodes: wire.selected_nodes,
        exact_sources: Box::new(ExactContextSources {
            selected_executions: wire.selected_executions,
            selected_workspace_values: wire.selected_workspace_values,
            explicit_evidence: wire.explicit_evidence,
        }),
        selected_roles: wire.selected_roles,
        include_categories: wire.include_categories,
        exclude_categories: wire.exclude_categories,
        artifact_selector: wire.artifact_selector,
        budget: wire.budget,
        ordering: wire.ordering,
        truncation: wire.truncation,
        session: wire.session,
        fail_closed: wire.fail_closed,
    };
    policy.validate().map(|()| policy)
});

impl Default for TaskContextPolicy {
    fn default() -> Self {
        Self {
            include_direct_inputs: true,
            ancestor_depth: None,
            selected_nodes: BTreeSet::new(),
            exact_sources: Box::default(),
            selected_roles: BTreeSet::new(),
            include_categories: BTreeSet::from([ContextCategory::DirectInput]),
            exclude_categories: BTreeSet::from([
                ContextCategory::RawProgress,
                ContextCategory::ToolTrace,
                ContextCategory::VerboseCommandOutput,
                ContextCategory::PriorPrompt,
            ]),
            artifact_selector: None,
            budget: ContextBudget::default(),
            ordering: ContextOrdering::default(),
            truncation: ContextTruncation::default(),
            session: ContextSessionPolicy::default(),
            fail_closed: true,
        }
    }
}

impl TaskContextPolicy {
    /// Constructs a validated policy from explicit semantic facts.
    #[allow(clippy::too_many_arguments)] // One validated blueprint contract keeps every semantic selector fact explicit.
    pub fn new(
        include_direct_inputs: bool,
        ancestor_depth: Option<u16>,
        selected_nodes: BTreeSet<NodeId>,
        selected_roles: BTreeSet<ContextSemanticRole>,
        include_categories: BTreeSet<ContextCategory>,
        exclude_categories: BTreeSet<ContextCategory>,
        artifact_selector: Option<ContextArtifactSelector>,
        budget: ContextBudget,
        ordering: ContextOrdering,
        truncation: ContextTruncation,
        session: ContextSessionPolicy,
        fail_closed: bool,
    ) -> Result<Self, ModelError> {
        let policy = Self {
            include_direct_inputs,
            ancestor_depth,
            selected_nodes,
            exact_sources: Box::default(),
            selected_roles,
            include_categories,
            exclude_categories,
            artifact_selector,
            budget,
            ordering,
            truncation,
            session,
            fail_closed,
        };
        policy.validate()?;
        Ok(policy)
    }

    /// Whether exact declared task inputs are eligible.
    #[must_use]
    pub const fn include_direct_inputs(&self) -> bool {
        self.include_direct_inputs
    }

    /// Maximum incoming graph-edge depth, when ancestry is enabled.
    #[must_use]
    pub const fn ancestor_depth(&self) -> Option<u16> {
        self.ancestor_depth
    }

    /// Explicitly selected semantic node identities.
    #[must_use]
    pub const fn selected_nodes(&self) -> &BTreeSet<NodeId> {
        &self.selected_nodes
    }

    /// Exact execution-occurrence identities selected in addition to semantic nodes.
    #[must_use]
    pub const fn selected_executions(&self) -> &BTreeSet<String> {
        &self.exact_sources.selected_executions
    }

    /// Exact canonical workspace-value references selected by policy.
    #[must_use]
    pub const fn selected_workspace_values(&self) -> &BTreeSet<String> {
        &self.exact_sources.selected_workspace_values
    }

    /// Explicit bounded durable evidence identities supplied by workflow/operator policy.
    #[must_use]
    pub const fn explicit_evidence(&self) -> &BTreeSet<String> {
        &self.exact_sources.explicit_evidence
    }

    /// Adds exact execution, workspace-value, and evidence selectors.
    pub fn with_exact_sources(
        mut self,
        selected_executions: BTreeSet<String>,
        selected_workspace_values: BTreeSet<String>,
        explicit_evidence: BTreeSet<String>,
    ) -> Result<Self, ModelError> {
        validate_exact_source_set("context.selected_executions", &selected_executions)?;
        validate_exact_source_set(
            "context.selected_workspace_values",
            &selected_workspace_values,
        )?;
        validate_exact_source_set("context.explicit_evidence", &explicit_evidence)?;
        self.exact_sources = Box::new(ExactContextSources {
            selected_executions,
            selected_workspace_values,
            explicit_evidence,
        });
        Ok(self)
    }

    /// Explicitly selected known semantic roles.
    #[must_use]
    pub const fn selected_roles(&self) -> &BTreeSet<ContextSemanticRole> {
        &self.selected_roles
    }

    /// Included semantic categories.
    #[must_use]
    pub const fn include_categories(&self) -> &BTreeSet<ContextCategory> {
        &self.include_categories
    }

    /// Categories excluded even when another selector matches.
    #[must_use]
    pub const fn exclude_categories(&self) -> &BTreeSet<ContextCategory> {
        &self.exclude_categories
    }

    /// Optional artifact metadata filter.
    #[must_use]
    pub const fn artifact_selector(&self) -> Option<&ContextArtifactSelector> {
        self.artifact_selector.as_ref()
    }

    /// Hard selection budget.
    #[must_use]
    pub const fn budget(&self) -> ContextBudget {
        self.budget
    }

    /// Stable ordering policy.
    #[must_use]
    pub const fn ordering(&self) -> ContextOrdering {
        self.ordering
    }

    /// Stable overflow handling.
    #[must_use]
    pub const fn truncation(&self) -> ContextTruncation {
        self.truncation
    }

    /// Explicit provider-session policy.
    #[must_use]
    pub const fn session(&self) -> ContextSessionPolicy {
        self.session
    }

    /// Whether unresolved required context rejects dispatch.
    #[must_use]
    pub const fn fail_closed(&self) -> bool {
        self.fail_closed
    }

    /// Domain-separated digest of the canonical policy bytes.
    pub fn digest(&self) -> Result<ContentDigest, ModelError> {
        let bytes = canonical_json_bytes(self, POLICY_JSON_LIMITS)
            .map_err(|error| ModelError::new("context", format!("{error:?}")))?;
        let mut hasher = blake3::Hasher::new();
        hasher.update(b"milkdrift.context-policy.v1\0");
        hasher.update(&bytes);
        Ok(ContentDigest::from_hash(hasher.finalize()))
    }

    fn validate(&self) -> Result<(), ModelError> {
        if self.ancestor_depth == Some(0) {
            return Err(ModelError::new(
                "context.ancestor_depth",
                "maximum ancestor depth must be nonzero when supplied",
            ));
        }
        if self.selected_nodes.len() > MAX_SELECTORS || self.selected_roles.len() > MAX_SELECTORS {
            return Err(ModelError::new(
                "context.selectors",
                "at most 256 exact nodes and roles are supported",
            ));
        }
        validate_exact_source_set(
            "context.selected_executions",
            &self.exact_sources.selected_executions,
        )?;
        validate_exact_source_set(
            "context.selected_workspace_values",
            &self.exact_sources.selected_workspace_values,
        )?;
        validate_exact_source_set(
            "context.explicit_evidence",
            &self.exact_sources.explicit_evidence,
        )?;
        if self.include_categories.len() > MAX_SELECTORS
            || self.exclude_categories.len() > MAX_SELECTORS
        {
            return Err(ModelError::new(
                "context.categories",
                "category selector cardinality exceeds the supported bound",
            ));
        }
        if self
            .include_categories
            .iter()
            .any(|category| self.exclude_categories.contains(category))
        {
            return Err(ModelError::new(
                "context.categories",
                "a category cannot be both included and excluded",
            ));
        }
        self.budget.validate()
    }
}

fn validate_text_set(location: &'static str, values: &BTreeSet<String>) -> Result<(), ModelError> {
    if values.len() > MAX_SELECTORS
        || values
            .iter()
            .any(|value| value.is_empty() || value.len() > MAX_SELECTOR_TEXT_BYTES)
    {
        return Err(ModelError::new(
            location,
            "selector values must contain 1..=255 bytes and at most 256 entries",
        ));
    }
    Ok(())
}

fn validate_exact_source_set(
    location: &'static str,
    values: &BTreeSet<String>,
) -> Result<(), ModelError> {
    if values.len() > MAX_SELECTORS
        || values
            .iter()
            .any(|value| value.is_empty() || value.len() > MAX_EXACT_SOURCE_BYTES)
    {
        return Err(ModelError::new(
            location,
            "exact source values must contain 1..=1024 bytes and at most 256 entries",
        ));
    }
    Ok(())
}
