use milkdrift_authority::{ActorRef, GrantDigest, GrantId};
use milkdrift_blueprint::{RevisionId, WorkflowId};
use milkdrift_workspace::RunId;
use serde::{Deserialize, Serialize};

use crate::{CommandId, IntegrityDigest, PageSize, PersistenceError, RunSequence, TimestampMillis};

/// Current durable external-command receipt schema.
pub const APPLICATION_COMMAND_RECEIPT_SCHEMA_VERSION_V1: u32 = 1;
/// Current durable presentation-layout record schema.
pub const APPLICATION_LAYOUT_RECORD_SCHEMA_VERSION_V1: u32 = 1;
/// Maximum canonical result bytes retained in an external receipt.
const MAX_APPLICATION_COMMAND_RESULT_BYTES: usize = 1_310_720;
/// Maximum canonical layout bytes retained in one independently addressed record.
const MAX_APPLICATION_LAYOUT_BYTES: usize = 262_144;
/// Maximum opaque continuation bytes returned by application-state pages.
const MAX_APPLICATION_CURSOR_BYTES: usize = 1_024;

/// Durable reference to the authoritative effect or read identity of a command.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case", tag = "type", deny_unknown_fields)]
pub enum ApplicationEffectReference {
    /// An immutable semantic revision was validated or stored.
    Revision {
        /// Exact immutable revision that the command addressed.
        revision: RevisionId,
    },
    /// A runtime transaction committed through an exact aggregate sequence.
    RunSequence {
        /// Aggregate owning the committed events.
        run: RunId,
        /// Durable head after applying the command.
        resulting_sequence: RunSequence,
    },
    /// Presentation-only layout state was committed outside semantic identity.
    Layout {
        /// Workflow whose presentation changed.
        workflow: WorkflowId,
        /// Semantic revision associated with the layout.
        revision: RevisionId,
        /// Optimistic layout generation committed by the command.
        generation: u64,
        /// Digest of the committed layout bytes.
        digest: IntegrityDigest,
    },
    /// A proposal became discoverable; exact state remains owned by control/runtime facts.
    Proposal {
        /// Run whose prospective repair was proposed.
        run: RunId,
        /// Stable proposal discovery identity.
        proposal: String,
        /// Immutable revision proposed for future execution.
        proposed_revision: RevisionId,
    },
}

impl ApplicationEffectReference {
    fn validate(&self) -> Result<(), PersistenceError> {
        match self {
            Self::Layout { generation, .. } if *generation == 0 => {
                Err(PersistenceError::InvalidDocument(
                    "application layout effect generation must be nonzero".to_owned(),
                ))
            }
            Self::Proposal { proposal, .. } => {
                validate_identity_text(proposal, "application_proposal_identity")
            }
            _ => Ok(()),
        }
    }
}

/// Exact bounded result retained for replay, including intentional deterministic rejection.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case", tag = "disposition", deny_unknown_fields)]
pub enum ApplicationCommandResult {
    /// The command was accepted; the canonical response is retained exactly.
    Accepted {
        /// Exact canonical response bytes replayed after a lost reply.
        document: Vec<u8>,
        /// Authoritative effect or read identity, when applicable.
        effect: Option<ApplicationEffectReference>,
    },
    /// The command was deterministically rejected under the recorded authority/validation basis.
    Rejected {
        /// Exact canonical rejection bytes retained for replay.
        document: Vec<u8>,
    },
}

impl ApplicationCommandResult {
    /// Returns the exact canonical result document.
    #[must_use]
    pub fn document(&self) -> &[u8] {
        match self {
            Self::Accepted { document, .. } | Self::Rejected { document } => document,
        }
    }

    /// Returns the durable effect reference for an accepted command, when one applies.
    #[must_use]
    pub const fn effect(&self) -> Option<&ApplicationEffectReference> {
        match self {
            Self::Accepted { effect, .. } => effect.as_ref(),
            Self::Rejected { .. } => None,
        }
    }

    /// Checks the exact result bytes and optional effect reference before a producer commits work.
    ///
    /// # Errors
    /// Refuses an empty or oversized result document or an invalid effect reference.
    pub fn validate(&self) -> Result<(), PersistenceError> {
        if self.document().is_empty()
            || self.document().len() > MAX_APPLICATION_COMMAND_RESULT_BYTES
        {
            return Err(PersistenceError::Bounds {
                location: "application_command_result",
                reason: format!(
                    "must contain 1..={MAX_APPLICATION_COMMAND_RESULT_BYTES} canonical bytes"
                ),
            });
        }
        if let Some(effect) = self.effect() {
            effect.validate()?;
        }
        Ok(())
    }
}

/// Retained answer to one actor-scoped external command, including a durable rejection.
///
/// The digest binds the complete canonical request; the result preserves the original
/// response and any authoritative effect reference. This is separate from a runtime
/// [`crate::CommandReceipt`], whose intent can exclude delivery metadata. Hot/cold
/// archival preserves the same document so a lost reply remains recoverable.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ApplicationCommandReceipt {
    schema_version: u32,
    actor: ActorRef,
    command: CommandId,
    command_schema_version: u32,
    command_digest: IntegrityDigest,
    grant: GrantId,
    grant_revision: u64,
    grant_digest: GrantDigest,
    authority_decision_digest: Option<String>,
    created_at: TimestampMillis,
    completed_at: TimestampMillis,
    result: ApplicationCommandResult,
}

impl ApplicationCommandReceipt {
    /// Constructs and validates one exact external command result.
    ///
    /// # Errors
    /// Refuses the receipt versions, times, decision digest or result rejected by [`Self::validate`].
    #[expect(
        clippy::too_many_arguments,
        reason = "Durable replay binds actor and exact command to the grant decision, completion times and immutable result"
    )]
    pub fn new(
        actor: ActorRef,
        command: CommandId,
        command_schema_version: u32,
        command_digest: IntegrityDigest,
        grant: GrantId,
        grant_revision: u64,
        grant_digest: GrantDigest,
        authority_decision_digest: Option<String>,
        created_at: TimestampMillis,
        completed_at: TimestampMillis,
        result: ApplicationCommandResult,
    ) -> Result<Self, PersistenceError> {
        let receipt = Self {
            schema_version: APPLICATION_COMMAND_RECEIPT_SCHEMA_VERSION_V1,
            actor,
            command,
            command_schema_version,
            command_digest,
            grant,
            grant_revision,
            grant_digest,
            authority_decision_digest,
            created_at,
            completed_at,
            result,
        };
        receipt.validate()?;
        Ok(receipt)
    }

    /// Verifies stored receipt invariants after decoding.
    ///
    /// # Errors
    /// Refuses unsupported receipt schema, zero command schema or grant revision,
    /// completion before creation, a malformed decision digest, an empty or oversized
    /// result, or invalid layout/proposal effect metadata.
    pub fn validate(&self) -> Result<(), PersistenceError> {
        if self.schema_version != APPLICATION_COMMAND_RECEIPT_SCHEMA_VERSION_V1 {
            return Err(PersistenceError::UnsupportedVersion {
                document: "application_command_receipt",
                found: self.schema_version,
                supported: APPLICATION_COMMAND_RECEIPT_SCHEMA_VERSION_V1,
            });
        }
        if self.command_schema_version == 0 || self.grant_revision == 0 {
            return Err(PersistenceError::InvalidDocument(
                "application receipt command schema and grant revision must be nonzero".to_owned(),
            ));
        }
        if self.completed_at < self.created_at {
            return Err(PersistenceError::InvalidDocument(
                "application receipt completion precedes creation".to_owned(),
            ));
        }
        if let Some(digest) = &self.authority_decision_digest {
            validate_digest_text(digest, "authority decision digest")?;
        }
        self.result.validate()
    }

    /// Authenticated actor and receipt-key scope.
    #[must_use]
    pub const fn actor(&self) -> &ActorRef {
        &self.actor
    }
    /// Client-owned idempotency identity.
    #[must_use]
    pub const fn command(&self) -> &CommandId {
        &self.command
    }
    /// Canonical command digest, including its versioned envelope.
    #[must_use]
    pub const fn command_digest(&self) -> &IntegrityDigest {
        &self.command_digest
    }
    /// Exact durable accepted/rejected result.
    #[must_use]
    pub const fn result(&self) -> &ApplicationCommandResult {
        &self.result
    }
    /// Exact immutable grant identity used at application entry.
    #[must_use]
    pub const fn grant(&self) -> &GrantId {
        &self.grant
    }
    /// Exact grant revision used at application entry.
    #[must_use]
    pub const fn grant_revision(&self) -> u64 {
        self.grant_revision
    }
    /// Exact grant digest used at application entry.
    #[must_use]
    pub const fn grant_digest(&self) -> &GrantDigest {
        &self.grant_digest
    }
    /// Boundary decision digest when application ownership produced one directly.
    #[must_use]
    pub fn authority_decision_digest(&self) -> Option<&str> {
        self.authority_decision_digest.as_deref()
    }
    /// Receipt creation time.
    #[must_use]
    pub const fn created_at(&self) -> TimestampMillis {
        self.created_at
    }
    /// Receipt completion time.
    #[must_use]
    pub const fn completed_at(&self) -> TimestampMillis {
        self.completed_at
    }
}

/// Opaque stable continuation for one application-state family.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ApplicationCursor(Vec<u8>);

impl ApplicationCursor {
    /// Validates bounded non-empty adapter-owned cursor bytes.
    ///
    /// # Errors
    /// Refuses empty cursors and cursors longer than 1,024 bytes.
    pub fn new(value: Vec<u8>) -> Result<Self, PersistenceError> {
        if value.is_empty() || value.len() > MAX_APPLICATION_CURSOR_BYTES {
            return Err(PersistenceError::InvalidCursor(format!(
                "application cursor must contain 1..={MAX_APPLICATION_CURSOR_BYTES} bytes"
            )));
        }
        Ok(Self(value))
    }
    /// Returns opaque adapter-owned bytes.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

/// Bounded application-state page request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ApplicationPageQuery {
    /// Exclusive continuation from a prior page.
    pub after: Option<ApplicationCursor>,
    /// Nonzero global page size.
    pub limit: PageSize,
}

/// Bounded application-state page.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ApplicationPage<T> {
    /// Verified page items.
    pub items: Vec<T>,
    /// Exclusive continuation, absent at the observed end.
    pub next: Option<ApplicationCursor>,
}

/// Current durable accounting and archival boundary for application receipts.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ApplicationReceiptStatus {
    /// Complete immutable receipts in the bounded operational tier.
    pub hot_count: u64,
    /// Complete immutable receipts in transparent cold storage.
    pub cold_count: u64,
    /// Configured maximum number of hot receipt documents.
    pub hot_bound: u32,
    /// Maximum receipts moved by one explicit or transactional archival batch.
    pub archive_batch_size: u32,
    /// Monotonic successful archival transaction generation.
    pub archive_generation: u64,
    /// Boundary time supplied by the successful archival owner, absent before the first move.
    pub last_archived_at: Option<TimestampMillis>,
}

/// One bounded oldest-first archival request.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ApplicationReceiptArchiveRequest {
    /// Exact archival generation observed immediately before this request.
    pub expected_generation: u64,
    /// Boundary time recorded if at least one receipt is moved.
    pub archived_at: TimestampMillis,
}

/// Result of one bounded transactional archival request.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ApplicationReceiptArchiveOutcome {
    /// Number of complete immutable receipt documents moved from hot to cold.
    pub archived: u32,
    /// Accounting and successful archival boundary after the transaction.
    pub status: ApplicationReceiptStatus,
}

/// Presentation-only layout update supplied to the atomic command transaction.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ApplicationLayoutUpdate {
    /// Independent layout schema.
    pub layout_schema_version: u32,
    /// Workflow association.
    pub workflow: WorkflowId,
    /// Optional semantic revision/view association; currently always exact revision.
    pub revision: RevisionId,
    /// Optimistic generation supplied by the caller.
    pub generation: u64,
    /// Digest of exact canonical layout bytes.
    pub digest: IntegrityDigest,
    /// Authenticated author provenance.
    pub author: ActorRef,
    /// Application boundary update time.
    pub updated_at: TimestampMillis,
    /// Exact independently versioned layout document.
    pub document: Vec<u8>,
}

impl ApplicationLayoutUpdate {
    /// Verifies update bounds before adapter entry.
    ///
    /// # Errors
    /// Refuses zero schema or generation, and documents outside 1–262,144 bytes.
    pub fn validate(&self) -> Result<(), PersistenceError> {
        if self.layout_schema_version == 0 || self.generation == 0 {
            return Err(PersistenceError::InvalidDocument(
                "layout schema and generation must be nonzero".to_owned(),
            ));
        }
        if self.document.is_empty() || self.document.len() > MAX_APPLICATION_LAYOUT_BYTES {
            return Err(PersistenceError::Bounds {
                location: "application_layout",
                reason: format!("must contain 1..={MAX_APPLICATION_LAYOUT_BYTES} bytes"),
            });
        }
        Ok(())
    }
}

/// Durable independently addressed presentation layout with provenance timestamps.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ApplicationLayout {
    schema_version: u32,
    layout_schema_version: u32,
    workflow: WorkflowId,
    revision: RevisionId,
    generation: u64,
    digest: IntegrityDigest,
    author: ActorRef,
    created_at: TimestampMillis,
    updated_at: TimestampMillis,
    document: Vec<u8>,
}

impl ApplicationLayout {
    /// Constructs a stored record; adapters preserve `created_at` across replacements.
    ///
    /// # Errors
    /// Refuses an invalid update or an update timestamp preceding `created_at`.
    pub fn from_update(
        update: ApplicationLayoutUpdate,
        created_at: TimestampMillis,
    ) -> Result<Self, PersistenceError> {
        update.validate()?;
        let value = Self {
            schema_version: APPLICATION_LAYOUT_RECORD_SCHEMA_VERSION_V1,
            layout_schema_version: update.layout_schema_version,
            workflow: update.workflow,
            revision: update.revision,
            generation: update.generation,
            digest: update.digest,
            author: update.author,
            created_at,
            updated_at: update.updated_at,
            document: update.document,
        };
        value.validate()?;
        Ok(value)
    }

    /// Verifies stored layout invariants after decoding.
    ///
    /// # Errors
    /// Refuses unsupported record schema, zero layout schema/generation, reversed
    /// timestamps, or retained documents outside 1–262,144 bytes.
    pub fn validate(&self) -> Result<(), PersistenceError> {
        if self.schema_version != APPLICATION_LAYOUT_RECORD_SCHEMA_VERSION_V1 {
            return Err(PersistenceError::UnsupportedVersion {
                document: "application_layout",
                found: self.schema_version,
                supported: APPLICATION_LAYOUT_RECORD_SCHEMA_VERSION_V1,
            });
        }
        if self.layout_schema_version == 0
            || self.generation == 0
            || self.updated_at < self.created_at
        {
            return Err(PersistenceError::InvalidDocument(
                "stored layout version/generation/timestamps are invalid".to_owned(),
            ));
        }
        if self.document.is_empty() || self.document.len() > MAX_APPLICATION_LAYOUT_BYTES {
            return Err(PersistenceError::Corruption(
                "stored layout document violates its byte bound".to_owned(),
            ));
        }
        Ok(())
    }

    /// Workflow association.
    #[must_use]
    pub const fn workflow(&self) -> &WorkflowId {
        &self.workflow
    }
    /// Exact revision/view association.
    #[must_use]
    pub const fn revision(&self) -> &RevisionId {
        &self.revision
    }
    /// Optimistic generation.
    #[must_use]
    pub const fn generation(&self) -> u64 {
        self.generation
    }
    /// Layout content digest.
    #[must_use]
    pub const fn digest(&self) -> &IntegrityDigest {
        &self.digest
    }
    /// Authenticated author.
    #[must_use]
    pub const fn author(&self) -> &ActorRef {
        &self.author
    }
    /// Creation time preserved across updates.
    #[must_use]
    pub const fn created_at(&self) -> TimestampMillis {
        self.created_at
    }
    /// Last update time.
    #[must_use]
    pub const fn updated_at(&self) -> TimestampMillis {
        self.updated_at
    }
    /// Exact canonical layout bytes.
    #[must_use]
    pub fn document(&self) -> &[u8] {
        &self.document
    }
}

/// First-class durable proposal discovery entry derived from an accepted receipt.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProposalIndexEntry {
    /// Owning run.
    pub run: RunId,
    /// Stable proposal identity.
    pub proposal: String,
    /// Immutable prospective revision.
    pub proposed_revision: RevisionId,
    /// Receipt-key actor.
    pub receipt_actor: ActorRef,
    /// Receipt-key command.
    pub receipt_command: CommandId,
    /// Discovery creation time.
    pub created_at: TimestampMillis,
}

impl ProposalIndexEntry {
    /// Verifies the bounded identity fields.
    ///
    /// # Errors
    /// Refuses a proposal identity outside 1–192 portable ASCII bytes or without an
    /// alphanumeric prefix.
    pub fn validate(&self) -> Result<(), PersistenceError> {
        validate_identity_text(&self.proposal, "proposal_index_identity")
    }
}

/// Atomic same-store application effect accompanying an external receipt.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ApplicationCommandEffect {
    /// The effect is owned by another idempotent transaction, or the command is read-only.
    None,
    /// Commit layout state and its receipt in one redb transaction.
    PutLayout(ApplicationLayoutUpdate),
    /// Commit the proposal discovery projection and receipt together.
    IndexProposal(ProposalIndexEntry),
}

/// One receipt plus the only same-store effect allowed in its transaction.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ApplicationCommandCommit {
    /// Exact accepted/rejected receipt.
    pub receipt: ApplicationCommandReceipt,
    /// Narrow optional same-store effect.
    pub effect: ApplicationCommandEffect,
}

impl ApplicationCommandCommit {
    /// Verifies that receipt references and same-store effects agree exactly.
    ///
    /// # Errors
    /// Refuses invalid receipts, layouts or proposal entries and any disagreement
    /// between the recorded effect reference and the effect to commit atomically.
    pub fn validate(&self) -> Result<(), PersistenceError> {
        self.receipt.validate()?;
        match (&self.effect, self.receipt.result().effect()) {
            (
                ApplicationCommandEffect::None,
                None
                | Some(ApplicationEffectReference::Revision { .. })
                | Some(ApplicationEffectReference::RunSequence { .. }),
            ) => Ok(()),
            (
                ApplicationCommandEffect::PutLayout(layout),
                Some(ApplicationEffectReference::Layout {
                    workflow,
                    revision,
                    generation,
                    digest,
                }),
            ) if workflow == &layout.workflow
                && revision == &layout.revision
                && *generation == layout.generation
                && digest == &layout.digest =>
            {
                layout.validate()
            }
            (
                ApplicationCommandEffect::IndexProposal(entry),
                Some(ApplicationEffectReference::Proposal {
                    run,
                    proposal,
                    proposed_revision,
                }),
            ) if run == &entry.run
                && proposal == &entry.proposal
                && proposed_revision == &entry.proposed_revision
                && self.receipt.actor() == &entry.receipt_actor
                && self.receipt.command() == &entry.receipt_command =>
            {
                entry.validate()
            }
            _ => Err(PersistenceError::InvalidDocument(
                "application receipt effect reference disagrees with its atomic effect".to_owned(),
            )),
        }
    }
}

/// Result of checking/inserting an external receipt transaction.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ApplicationCommandCommitOutcome {
    /// This transaction durably inserted the receipt and same-store effect.
    Committed,
    /// The exact receipt already existed; no write was performed.
    Replayed(Box<ApplicationCommandReceipt>),
}

/// Narrow external-command receipt/idempotency port.
///
/// A receipt belongs to exactly one tier for the store generation. Implementations must
/// look through hot and cold storage, verify saved records, and preserve exact responses
/// and conflict detection during archival. A storage error can follow commit: reread by
/// actor/command or redeliver the same commit before deciding what happened.
pub trait ApplicationCommandStore: Send + Sync {
    /// Reads one exact actor-scoped external receipt.
    ///
    /// # Errors
    /// Returns storage failures or corruption in the retained receipt or tier ownership.
    fn application_command_receipt(
        &self,
        actor: &ActorRef,
        command: &CommandId,
    ) -> Result<Option<ApplicationCommandReceipt>, PersistenceError>;
    /// Atomically checks/inserts a receipt and its optional same-store effect.
    ///
    /// Check existing identity/digest for replay or conflict before applying fresh effect
    /// guards. For a new identity, receipt insertion, required archival, and a supplied
    /// layout/proposal-index effect commit together. Runtime effects use a separate
    /// idempotent journal transaction; the application must recover that result if its
    /// receipt was not saved. This port cannot repeat the runtime operation on its own.
    ///
    /// # Errors
    /// Refuses invalid commits, conflicting replay keys or stale effect guards; returns
    /// storage, corruption and archival-capacity failures. Reread after an ambiguous write.
    fn commit_application_command(
        &self,
        commit: &ApplicationCommandCommit,
    ) -> Result<ApplicationCommandCommitOutcome, PersistenceError>;
    /// Lists receipts in stable bounded key order for administration/recovery.
    /// Pagination spans both tiers; moving a receipt must not change its logical cursor key.
    ///
    /// # Errors
    /// Refuses invalid continuations and returns storage or retained receipt corruption.
    fn application_command_receipts(
        &self,
        query: &ApplicationPageQuery,
    ) -> Result<ApplicationPage<ApplicationCommandReceipt>, PersistenceError>;
    /// Returns exact hot/cold accounting and the most recent successful archival boundary.
    ///
    /// # Errors
    /// Returns storage failures or inconsistent retained accounting/generation evidence.
    fn application_receipt_status(&self) -> Result<ApplicationReceiptStatus, PersistenceError>;
    /// Moves at most the configured batch of oldest complete hot receipts to cold storage.
    /// The supplied archival generation guards the whole move. Preserve receipt bytes,
    /// remove hot ownership, and update counters/generation atomically; do not evict identity.
    ///
    /// # Errors
    /// Refuses stale archival generation and returns storage, capacity or receipt corruption.
    fn archive_application_command_receipts(
        &self,
        request: ApplicationReceiptArchiveRequest,
    ) -> Result<ApplicationReceiptArchiveOutcome, PersistenceError>;
}

/// Narrow independently addressed layout read/list port.
pub trait ApplicationLayoutStore: Send + Sync {
    /// Reads one exact workflow/revision layout.
    ///
    /// # Errors
    /// Returns storage failures or invalid retained layout records.
    fn application_layout(
        &self,
        workflow: &WorkflowId,
        revision: &RevisionId,
    ) -> Result<Option<ApplicationLayout>, PersistenceError>;
    /// Lists layouts in stable bounded key order.
    ///
    /// # Errors
    /// Refuses invalid continuations and returns storage failures or corrupt layout records.
    fn application_layouts(
        &self,
        query: &ApplicationPageQuery,
    ) -> Result<ApplicationPage<ApplicationLayout>, PersistenceError>;
}

/// First-class proposal discovery projection port.
///
/// Entries point back to authoritative accepted receipts. Rebuild checks both receipt
/// tiers and replaces only derived discovery; it does not recreate proposals or run
/// decisions. The daemon does not automatically rebuild this index during startup.
pub trait ProposalIndexStore: Send + Sync {
    /// Lists exact proposal identities for one run without scanning command receipts.
    ///
    /// # Errors
    /// Refuses invalid continuations and returns storage failures or inconsistent discovery records.
    fn proposal_index(
        &self,
        run: &RunId,
        query: &ApplicationPageQuery,
    ) -> Result<ApplicationPage<ProposalIndexEntry>, PersistenceError>;
    /// Validates and reconstructs the derived proposal index from authoritative receipts.
    ///
    /// # Errors
    /// Returns storage failures or invalid/conflicting receipt evidence; no proposal is fabricated.
    fn rebuild_proposal_index(&self) -> Result<u64, PersistenceError>;
}

/// Bounded decision facts appended for protected local operations and reads.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SecurityAuditEntry {
    /// Evaluation timestamp.
    pub evaluated_at: TimestampMillis,
    /// Authenticated actor.
    pub actor: ActorRef,
    /// Exact grant identity.
    pub grant: GrantId,
    /// Exact grant revision.
    pub grant_revision: u64,
    /// Exact grant digest.
    pub grant_digest: GrantDigest,
    /// Closed operation label.
    pub operation: String,
    /// Redacted digest of resource facts.
    pub resource_digest: IntegrityDigest,
    /// Exact decision digest.
    pub decision_digest: String,
    /// Stable allowed/denied outcome.
    pub outcome: String,
    /// Bounded stable reason codes.
    pub reason_codes: Vec<String>,
}

impl SecurityAuditEntry {
    /// Verifies fields before append or after decoding.
    ///
    /// # Errors
    /// Refuses zero grant revision, more than 64 reasons, invalid operation/outcome/reason
    /// identities, or a noncanonical decision digest.
    pub fn validate(&self) -> Result<(), PersistenceError> {
        if self.grant_revision == 0 || self.reason_codes.len() > 64 {
            return Err(PersistenceError::InvalidDocument(
                "security audit grant revision/reason count is invalid".to_owned(),
            ));
        }
        validate_identity_text(&self.operation, "security_audit_operation")?;
        validate_identity_text(&self.outcome, "security_audit_outcome")?;
        validate_digest_text(&self.decision_digest, "security audit decision digest")?;
        for reason in &self.reason_codes {
            validate_identity_text(reason, "security_audit_reason")?;
        }
        Ok(())
    }
}

/// One sequenced retained security decision record.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SecurityAuditRecord {
    /// Monotonic append sequence; gaps can result from bounded prefix retention.
    pub sequence: u64,
    /// Exact bounded decision facts.
    pub entry: SecurityAuditEntry,
}

/// Narrow append-only bounded security-audit port.
pub trait SecurityAuditStore: Send + Sync {
    /// Appends one decision, atomically evicting only the oldest audit row at its independent
    /// retention bound. Command receipts are never evicted through this port.
    ///
    /// # Errors
    /// Refuses invalid entries and returns storage, sequence overflow or retained audit corruption.
    fn append_security_audit(
        &self,
        entry: &SecurityAuditEntry,
    ) -> Result<SecurityAuditRecord, PersistenceError>;
    /// Lists retained audit rows in stable sequence order.
    ///
    /// # Errors
    /// Refuses invalid continuations and returns storage failures or corrupt audit records.
    fn security_audit(
        &self,
        query: &ApplicationPageQuery,
    ) -> Result<ApplicationPage<SecurityAuditRecord>, PersistenceError>;
}

fn validate_identity_text(value: &str, location: &'static str) -> Result<(), PersistenceError> {
    if value.is_empty()
        || value.len() > 192
        || !value.is_ascii()
        || !value
            .as_bytes()
            .first()
            .is_some_and(u8::is_ascii_alphanumeric)
        || !value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':' | b'/')
        })
    {
        return Err(PersistenceError::Bounds {
            location,
            reason: "must be a 1..=192 byte portable identity".to_owned(),
        });
    }
    Ok(())
}

fn validate_digest_text(value: &str, label: &str) -> Result<(), PersistenceError> {
    IntegrityDigest::new(value.to_owned())
        .map(|_| ())
        .map_err(|_| {
            PersistenceError::InvalidDocument(format!("{label} is not a canonical BLAKE3 digest"))
        })
}
