use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use milkdrift_contracts::{JsonLimits, canonical_json_bytes};

use crate::{ContentDigest, ModelError, NodeId};

const MAX_SELECTORS: usize = 256;
const MAX_SELECTOR_TEXT_BYTES: usize = 255;
const POLICY_JSON_LIMITS: JsonLimits = JsonLimits {
    maximum_depth: 16,
    maximum_string_bytes: MAX_SELECTOR_TEXT_BYTES,
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
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ContextArtifactSelector {
    names: BTreeSet<String>,
    media_types: BTreeSet<String>,
    sensitivities: BTreeSet<ContextArtifactSensitivity>,
    retentions: BTreeSet<ContextArtifactRetention>,
    provenance: BTreeSet<ContextProvenanceClass>,
}

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
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
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
}

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
        })
    }
}

impl Default for ContextBudget {
    fn default() -> Self {
        Self {
            max_items: 64,
            max_bytes: 262_144,
            max_artifact_bytes: 16_777_216,
            max_model_input_units: None,
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
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TaskContextPolicy {
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
}

impl Default for TaskContextPolicy {
    fn default() -> Self {
        Self {
            include_direct_inputs: true,
            ancestor_depth: None,
            selected_nodes: BTreeSet::new(),
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
    #[allow(clippy::too_many_arguments)]
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
        if ancestor_depth == Some(0) {
            return Err(ModelError::new(
                "context.ancestor_depth",
                "maximum ancestor depth must be nonzero when supplied",
            ));
        }
        if selected_nodes.len() > MAX_SELECTORS || selected_roles.len() > MAX_SELECTORS {
            return Err(ModelError::new(
                "context.selectors",
                "at most 256 exact nodes and roles are supported",
            ));
        }
        if include_categories.len() > MAX_SELECTORS || exclude_categories.len() > MAX_SELECTORS {
            return Err(ModelError::new(
                "context.categories",
                "category selector cardinality exceeds the supported bound",
            ));
        }
        if include_categories
            .iter()
            .any(|category| exclude_categories.contains(category))
        {
            return Err(ModelError::new(
                "context.categories",
                "a category cannot be both included and excluded",
            ));
        }
        ContextBudget::new(
            budget.max_items,
            budget.max_bytes,
            budget.max_artifact_bytes,
            budget.max_model_input_units,
        )?;
        Ok(Self {
            include_direct_inputs,
            ancestor_depth,
            selected_nodes,
            selected_roles,
            include_categories,
            exclude_categories,
            artifact_selector,
            budget,
            ordering,
            truncation,
            session,
            fail_closed,
        })
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
