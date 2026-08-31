use super::*;

/// Daemon liveness/readiness/draining state.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct HealthRead {
    /// Stable lifecycle state.
    pub state: DaemonState,
    /// Process is alive.
    pub live: bool,
    /// Recovery completed and commands may be considered.
    pub ready: bool,
    /// New mutations are refused.
    pub draining: bool,
    /// Current request queue occupancy.
    pub queued_requests: u32,
    /// Fixed request queue bound.
    pub request_queue_capacity: u32,
    /// Current effect workers executing external work.
    pub active_effects: u32,
    /// Redacted last startup/worker failure.
    pub last_failure: Option<String>,
    /// Bounded application-receipt lifecycle facts without command or result content.
    pub application_receipts: ApplicationReceiptHealthRead,
    /// Serving-peer active/hot/tombstone lifecycle facts without per-peer identities.
    pub peer_executions: PeerExecutionHealthRead,
}

/// Redacted exact-replay receipt lifecycle health.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ApplicationReceiptHealthRead {
    /// Complete immutable receipts in the recent operational tier.
    pub hot_count: u64,
    /// Configured hot receipt ceiling.
    pub hot_bound: u32,
    /// Maximum receipts moved by one bounded archival transaction.
    pub archive_batch_size: u32,
    /// Complete immutable receipts in transparent cold storage.
    pub cold_count: u64,
    /// Monotonic successful archival boundary.
    pub archive_generation: u64,
    /// Time of the most recent successful move, absent before archival.
    pub last_archived_at_unix_ms: Option<u64>,
    /// Whether the last bounded archival/storage refresh failed.
    pub archival_degraded: bool,
    /// Redacted failure classification, never command or result content.
    pub last_archival_failure: Option<String>,
}

/// Redacted serving-peer execution retention and archival health.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PeerExecutionHealthRead {
    /// Whether serving-peer execution is enabled.
    pub enabled: bool,
    /// Accepted nonterminal execution count.
    pub active_count: u32,
    /// Configured global active ceiling.
    pub active_bound: u32,
    /// Active pre-entry dispatch queue count.
    pub dispatch_queued: u32,
    /// Configured durable dispatch queue ceiling.
    pub dispatch_bound: u32,
    /// Complete terminal/uncertain records retaining detailed observations.
    pub hot_terminal_count: u64,
    /// Configured hot terminal ceiling, including capacity reserved by active work.
    pub hot_terminal_bound: u64,
    /// Compact immutable archived request identities.
    pub tombstone_count: u64,
    /// Maximum records compacted in one transaction.
    pub archive_batch_size: u32,
    /// Minimum terminal age before detailed observations are compacted.
    pub observation_hot_retention_ms: u64,
    /// Monotonic successful archive generation.
    pub archive_generation: u64,
    /// Most recent successful archival boundary.
    pub last_archived_at_unix_ms: Option<u64>,
    /// Whether the last archival/status operation failed.
    pub archival_degraded: bool,
    /// Redacted archival failure classification.
    pub last_archival_failure: Option<String>,
}

/// Stable daemon lifecycle state.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DaemonState {
    /// Configuration and resources are being initialized.
    Starting,
    /// Recovery and adapter initialization completed.
    Ready,
    /// Shutdown has closed admission.
    Draining,
    /// Host has stopped its owned services.
    Stopped,
    /// Startup failed before readiness.
    Failed,
}

/// Compact immutable revision summary.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RevisionSummary {
    /// Revision identity.
    pub revision_id: String,
    /// Workflow lineage.
    pub workflow_id: String,
    /// User-facing lineage sequence.
    pub lineage_sequence: u64,
    /// Semantic digest.
    pub semantic_digest: String,
    /// Exact parents.
    pub parents: Vec<String>,
}

/// Bounded immutable revision inspection.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RevisionRead {
    /// Compact summary.
    pub summary: RevisionSummary,
    /// Bounded author reference.
    pub author: String,
    /// Bounded provenance reason.
    pub reason: String,
    /// Semantic node count.
    pub node_count: u32,
    /// Semantic edge count.
    pub edge_count: u32,
    /// Canonical portable document when explicitly requested.
    pub document: Option<Value>,
}

/// Structured bounded semantic difference.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RevisionDiffRead {
    /// Left revision.
    pub from_revision: String,
    /// Right revision.
    pub to_revision: String,
    /// Stable bounded changes.
    pub changes: Vec<RevisionChange>,
    /// True if further changes were omitted at the response bound.
    pub truncated: bool,
}

/// One stable semantic revision change.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RevisionChange {
    /// Added, removed, or changed.
    pub change: String,
    /// Node, edge, interface, or metadata.
    pub subject: String,
    /// Stable semantic identity where applicable.
    pub identity: Option<String>,
    /// Bounded structured summary.
    pub detail: Value,
}

/// Compact current run status.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RunRead {
    /// Run aggregate.
    pub run_id: String,
    /// Authoritative sequence.
    pub sequence: u64,
    /// Stable lifecycle label.
    pub lifecycle: String,
    /// Stable terminal outcome when lifecycle is terminal.
    pub terminal: Option<String>,
    /// Workflow lineage.
    pub workflow_id: Option<String>,
    /// Exact current revision.
    pub revision_id: Option<String>,
    /// Current semantic digest.
    pub semantic_digest: Option<String>,
    /// Compact execution frontier.
    pub nodes: Vec<NodeRead>,
    /// Unresolved side-effect obligations.
    pub uncertainty_count: u32,
}

/// Compact node-execution and latest-attempt state.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NodeRead {
    /// Logical execution identity.
    pub execution_id: String,
    /// Semantic node identity.
    pub node_id: String,
    /// Governing revision.
    pub revision_id: String,
    /// Stable execution state label.
    pub state: String,
    /// Total attempts.
    pub attempt_count: u32,
    /// Latest retained attempt.
    pub latest_attempt: Option<AttemptRead>,
}

/// Bounded attempt status and provenance.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AttemptRead {
    /// Attempt identity.
    pub attempt_id: String,
    /// Invocation identity after scheduling.
    pub invocation_id: Option<String>,
    /// Stable state label.
    pub state: String,
    /// Resolved capability identity.
    pub capability_id: Option<String>,
    /// Exact resolved capability descriptor revision.
    pub descriptor_revision: Option<u64>,
    /// Exact frozen capability-generation and safe implementation provenance.
    pub capability_provenance: Option<CapabilityProvenanceRead>,
    /// Frozen run-level actor, grant revision, policy, and accepted start decision.
    pub execution_authority: Option<ExecutionAuthorityRead>,
    /// Candidate-set authority decision made before capability resolution.
    pub resolution_authorization: Option<AuthorityDecisionRead>,
    /// Fresh exact-candidate authority decision made when the effect claim was acquired.
    pub claim_authorization: Option<AuthorityDecisionRead>,
    /// Final authority decision made immediately before adapter entry.
    pub entry_authorization: Option<AuthorityDecisionRead>,
    /// Exact provider profile selected for this attempt.
    pub provider_profile: Option<String>,
    /// Authenticated peer identity when remote execution recorded one.
    pub peer_id: Option<String>,
    /// Exact immutable context manifest.
    pub context_manifest: Option<ArtifactMetadataRead>,
    /// Authorized bounded contents of the exact context manifest.
    pub context: Option<ContextManifestRead>,
    /// `absent`, `metadata_only`, `authorized`, or `denied` without leaking contents.
    pub context_access: String,
    /// Stable terminal summary.
    pub terminal: Option<String>,
    /// Whether external outcome remains unresolved.
    pub uncertain: bool,
}

/// Bounded immutable run execution-authority provenance.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionAuthorityRead {
    /// Authenticated actor whose authority follows the work.
    pub actor_id: String,
    /// Exact immutable grant identity.
    pub grant_id: String,
    /// Exact immutable grant revision.
    pub grant_revision: u64,
    /// Digest of that grant revision.
    pub grant_digest: String,
    /// Acceptance-time revocation generation.
    pub revocation_generation: u64,
    /// Evaluator policy identity.
    pub policy_id: String,
    /// Evaluator policy version.
    pub policy_version: u32,
    /// Decision that established execution authority.
    pub accepted_decision_id: String,
    /// Digest of the accepted decision.
    pub accepted_decision_digest: String,
    /// Digest of the complete frozen basis.
    pub basis_digest: String,
}

/// Bounded immutable authority-decision provenance for one execution boundary.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AuthorityDecisionRead {
    /// Evaluated decision identity.
    pub decision_id: String,
    /// Actor carried into the evaluation.
    pub actor_id: String,
    /// Exact grant identity.
    pub grant_id: String,
    /// Exact grant revision.
    pub grant_revision: u64,
    /// Exact grant digest.
    pub grant_digest: String,
    /// Revocation generation evaluated at this boundary.
    pub revocation_generation: u64,
    /// Evaluator policy identity.
    pub policy_id: String,
    /// Evaluator policy version.
    pub policy_version: u32,
    /// Stable requested operation label.
    pub operation: String,
    /// True only for an allowed decision.
    pub allowed: bool,
    /// Digest of the complete decision snapshot.
    pub decision_digest: String,
}

/// Safe immutable capability-generation facts retained with an attempt.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CapabilityProvenanceRead {
    /// Domain-separated digest of the complete resolved capability snapshot.
    pub snapshot_digest: String,
    /// Exact execution isolation/trust class selected for the attempt.
    pub execution_trust: String,
    /// Local-process implementation identity, including safe path digests, when applicable.
    pub implementation_identity: Option<String>,
    /// Exact executable content digest, when the generation is a byte-pinned local process.
    pub implementation_content_digest: Option<String>,
    /// Exact executable byte size, when the generation is a byte-pinned local process.
    pub implementation_size_bytes: Option<u64>,
    /// Digest of the complete local-process profile, when applicable.
    pub process_profile_digest: Option<String>,
    /// Digest of execution-semantic local-process policy, when applicable.
    pub execution_policy_digest: Option<String>,
    /// Bounded operator-declared package/deployment revision, when supplied.
    pub package_revision: Option<String>,
    /// Bounded operator-declared implementation documentation reference, when supplied.
    pub documentation_reference: Option<String>,
}

/// Bounded authorized context policy, selection, omissions, and accounting.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ContextManifestRead {
    /// Inner context-manifest schema.
    pub schema_version: u32,
    /// Domain-separated canonical manifest digest.
    pub digest: String,
    /// Exact immutable task context policy.
    pub policy: Value,
    /// Selected entry metadata and causal provenance, capped by the read model.
    pub entries: Vec<Value>,
    /// Stable omission/denial/missing records, capped by the read model.
    pub omissions: Vec<Value>,
    /// Applied totals.
    pub totals: Value,
    /// Applied deterministic limits.
    pub budget: Value,
    /// Whether entry or omission detail exceeded the response cap.
    pub truncated: bool,
}

/// Public timeline category; never an internal event enum.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TimelineCategory {
    /// Run lifecycle changed.
    Lifecycle,
    /// Node became eligible/scheduled/started/terminal.
    Execution,
    /// Progress was durably observed.
    Progress,
    /// Artifact/output publication changed.
    Artifact,
    /// Signal or timer interaction.
    Coordination,
    /// Authority or approval decision.
    Authority,
    /// Recovery classified durable work.
    Recovery,
    /// Revision reconciliation changed.
    Reconciliation,
    /// External outcome remains uncertain.
    Uncertainty,
}

/// One bounded externally stable timeline entry.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TimelineEntry {
    /// Exact journal provenance sequence.
    pub sequence: u64,
    /// Boundary timestamp.
    pub timestamp_ms: u64,
    /// Stable projected category.
    pub category: TimelineCategory,
    /// Bounded actor reference.
    pub actor: String,
    /// Run aggregate.
    pub run_id: String,
    /// Optional related node.
    pub node_id: Option<String>,
    /// Optional related attempt.
    pub attempt_id: Option<String>,
    /// Optional related revision.
    pub revision_id: Option<String>,
    /// Stable non-secret summary.
    pub summary: String,
    /// Bounded category-specific detail.
    pub detail: Value,
}

/// Capability generation and health observation.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CapabilityRead {
    /// Capability identity.
    pub capability_id: String,
    /// Descriptor revision.
    pub generation: u64,
    /// Descriptor digest.
    pub descriptor_digest: String,
    /// Stable discovery category.
    pub category: String,
    /// Exact advertised operation identities.
    pub operations: Vec<String>,
    /// Optional provider/model profile identity.
    pub provider_profile: Option<String>,
    /// Stable execution-locality label.
    pub locality: String,
    /// Authenticated remote peer identity, when peer-owned.
    pub peer_id: Option<String>,
    /// Complete advertised trust-zone labels.
    pub trust_zones: Vec<String>,
    /// Exact execution-isolation/trust class.
    pub execution_trust: String,
    /// Whether this generation is selected for new work.
    pub current: bool,
    /// Whether new work is refused.
    pub draining: bool,
    /// Stable health label.
    pub health: String,
    /// Last known availability.
    pub available: Option<bool>,
    /// Active/maximum permits.
    pub active_permits: u32,
    /// Admission permit bound.
    pub permit_limit: u32,
}

/// Safe authenticated peer/catalog status without credentials or transport internals.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PeerRead {
    /// Configured authenticated remote peer identity.
    pub peer_id: String,
    /// Whether an authenticated session and live verified catalog are current.
    pub connected: bool,
    /// Stable redacted health classification.
    pub health: String,
    /// Current authenticated remote daemon session identity.
    pub session_id: Option<String>,
    /// Current verified catalog generation.
    pub catalog_generation: Option<u64>,
    /// Current verified catalog digest.
    pub catalog_digest: Option<String>,
    /// Current ordinary capability-host registration count.
    pub registered_capabilities: usize,
    /// Current catalog hard expiry, when connected.
    pub catalog_expires_at_unix_ms: Option<u64>,
    /// Whether the live relationship was administratively revoked.
    pub revoked: bool,
}

/// Safe artifact metadata without a filesystem path.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ArtifactMetadataRead {
    /// Artifact identity.
    pub artifact_id: String,
    /// Content digest.
    pub digest: String,
    /// Exact byte size.
    pub size: u64,
    /// Declared media type.
    pub content_type: String,
    /// Safe suggested download name.
    pub disposition_name: Option<String>,
    /// Stable sensitivity label.
    pub sensitivity: String,
}

/// Public actor/grant observation returned only under inspect authority.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AuthorityRead {
    /// Server-owned actor identity.
    pub actor: String,
    /// Exact configured grant identity.
    pub grant_id: String,
    /// Immutable grant revision.
    pub grant_revision: u64,
    /// Current revocation generation.
    pub revocation_generation: u64,
    /// Stable operation names allowed by configuration.
    pub operations: Vec<String>,
}

/// Workflow-controller proposal status.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProposalRead {
    /// Proposal identity.
    pub proposal_id: String,
    /// Exact prospective revision.
    pub proposed_revision: String,
    /// Stable reconciliation status.
    pub status: String,
    /// Whether approval is durably recorded.
    pub approved: bool,
    /// Optional application sequence.
    pub applied_sequence: Option<u64>,
}

/// A resumable, externally projected stream item.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ObservationEnvelope {
    /// Protocol version.
    pub protocol: ProtocolVersion,
    /// Feed-bound monotonically ordered cursor.
    pub cursor: Cursor,
    /// Observation timestamp, independent from heartbeats.
    pub observed_at_ms: u64,
    /// Exact filtered feed identity.
    pub feed: String,
    /// External read-model payload.
    pub observation: Observation,
}

/// Closed stream observation vocabulary.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(
    rename_all = "snake_case",
    tag = "type",
    content = "value",
    deny_unknown_fields
)]
#[allow(missing_docs)] // Variant prose documents each compact observation payload.
pub enum Observation {
    /// Projected timeline fact.
    Timeline(TimelineEntry),
    /// Compact current run state.
    RunStatus(RunRead),
    /// Capability generation/health change.
    Capability(CapabilityRead),
    /// Daemon lifecycle/health change.
    DaemonHealth(HealthRead),
    /// Server is shutting down this stream.
    StreamClosing { reason: String },
    /// Subscriber fell outside retained stream history.
    ResyncRequired { reason: String },
}
