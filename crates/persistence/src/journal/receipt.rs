use std::collections::BTreeSet;

use milkdrift_authority::{ActorRef, AuthorityDecisionSnapshot};
use milkdrift_capability::BoundedJson;
use milkdrift_workspace::RunId;
use serde::{Deserialize, Serialize};

use super::{
    COMMAND_RESULT_SCHEMA_VERSION_V1, COMMAND_RESULT_SCHEMA_VERSION_V2, MAX_COMMAND_DOCUMENT_BYTES,
    MAX_COMMAND_RESULT_DOCUMENT_BYTES,
};
use crate::{
    CommandId, IntegrityDigest, PersistenceError, RunSequence, TimestampMillis,
    bounded::MAX_EVENTS_PER_COMMIT,
};

/// Exact canonical runtime command receipt used for durable idempotency.
///
/// Persistence does not interpret runtime transitions. It retains the complete audit
/// document, the canonical semantic intent, and their derived fingerprint so a repeated
/// [`CommandId`] can be proven identical or conflicting across delivery retries.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommandReceipt {
    command: CommandId,
    run: RunId,
    actor: ActorRef,
    expected_sequence: RunSequence,
    submitted_at: TimestampMillis,
    canonical_document: Vec<u8>,
    canonical_intent: Vec<u8>,
    fingerprint: IntegrityDigest,
}

fn validate_canonical_command_bytes(
    location: &'static str,
    bytes: &[u8],
) -> Result<(), PersistenceError> {
    if bytes.is_empty() || bytes.len() > MAX_COMMAND_DOCUMENT_BYTES {
        return Err(PersistenceError::Bounds {
            location,
            reason: format!(
                "must contain 1..={MAX_COMMAND_DOCUMENT_BYTES} canonical document bytes"
            ),
        });
    }
    let value: serde_json::Value = serde_json::from_slice(bytes)?;
    let canonical = crate::document::canonical_json_bytes(&value, MAX_COMMAND_DOCUMENT_BYTES)?;
    if canonical != bytes {
        return Err(PersistenceError::InvalidDocument(format!(
            "{location} bytes must be canonical compact key-sorted JSON"
        )));
    }
    Ok(())
}

impl CommandReceipt {
    /// Constructs a receipt from a runtime-owned canonical command document.
    pub fn new(
        command: CommandId,
        run: RunId,
        actor: ActorRef,
        expected_sequence: RunSequence,
        submitted_at: TimestampMillis,
        canonical_document: Vec<u8>,
    ) -> Result<Self, PersistenceError> {
        Self::new_idempotent(
            command,
            run,
            actor,
            expected_sequence,
            submitted_at,
            canonical_document.clone(),
            canonical_document,
        )
    }

    /// Constructs a receipt whose idempotency fingerprint is bound to a canonical semantic
    /// intent document rather than retry-local delivery metadata.
    ///
    /// The complete command document is still retained for audit. Runtime callers use this
    /// constructor so an exact command can be retried at a newer optimistic sequence or
    /// timestamp without turning the same [`CommandId`] into a false conflict.
    #[allow(clippy::too_many_arguments)] // One validated durable document keeps its complete storage facts explicit.
    pub fn new_idempotent(
        command: CommandId,
        run: RunId,
        actor: ActorRef,
        expected_sequence: RunSequence,
        submitted_at: TimestampMillis,
        canonical_document: Vec<u8>,
        canonical_intent: Vec<u8>,
    ) -> Result<Self, PersistenceError> {
        validate_canonical_command_bytes("command.document", &canonical_document)?;
        validate_canonical_command_bytes("command.intent", &canonical_intent)?;

        let mut hasher = blake3::Hasher::new();
        hasher.update(b"milkdrift.command-receipt.semantic.v1\0");
        for component in [
            command.as_str().as_bytes(),
            run.as_str().as_bytes(),
            actor.as_str().as_bytes(),
            canonical_intent.as_slice(),
        ] {
            let length = u32::try_from(component.len()).map_err(|_| PersistenceError::Bounds {
                location: "command.receipt_component",
                reason: "component length does not fit u32".to_owned(),
            })?;
            hasher.update(&length.to_be_bytes());
            hasher.update(component);
        }
        let fingerprint = IntegrityDigest::new(format!("b3_{}", hasher.finalize()))?;
        Ok(Self {
            command,
            run,
            actor,
            expected_sequence,
            submitted_at,
            canonical_document,
            canonical_intent,
            fingerprint,
        })
    }

    /// Command/idempotency identity.
    #[must_use]
    pub const fn command(&self) -> &CommandId {
        &self.command
    }

    /// Owning aggregate.
    #[must_use]
    pub const fn run(&self) -> &RunId {
        &self.run
    }

    /// Issuer reference retained with the command.
    #[must_use]
    pub const fn actor(&self) -> &ActorRef {
        &self.actor
    }

    /// Optimistic guard supplied by the runtime.
    #[must_use]
    pub const fn expected_sequence(&self) -> RunSequence {
        self.expected_sequence
    }

    /// Boundary-clock receipt timestamp.
    #[must_use]
    pub const fn submitted_at(&self) -> TimestampMillis {
        self.submitted_at
    }

    /// Runtime-owned canonical command bytes retained as exact audit evidence.
    #[must_use]
    pub fn canonical_document(&self) -> &[u8] {
        &self.canonical_document
    }

    /// Canonical semantic intent bytes that own idempotency across delivery retries.
    ///
    /// These bytes are persisted separately from the complete audit document so storage
    /// can reconstruct and validate the semantic fingerprint without interpreting runtime
    /// fields such as the optimistic sequence or delivery timestamp.
    #[must_use]
    pub fn canonical_intent(&self) -> &[u8] {
        &self.canonical_intent
    }

    /// Domain-separated command fingerprint.
    #[must_use]
    pub const fn fingerprint(&self) -> &IntegrityDigest {
        &self.fingerprint
    }
}

/// Whether runtime transition validation accepted or rejected a command.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CommandDisposition {
    /// The command emitted one or more semantic events.
    Accepted,
    /// Runtime validation rejected it without semantic events.
    Rejected,
}

/// Fully durable typed result returned for exact command redelivery.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CommandResultDocument {
    schema_version: u32,
    command: CommandId,
    run: RunId,
    command_fingerprint: IntegrityDigest,
    disposition: CommandDisposition,
    resulting_sequence: RunSequence,
    event_ids: Vec<crate::EventId>,
    result: BoundedJson,
    #[serde(skip_serializing_if = "Option::is_none")]
    authorization: Option<AuthorityDecisionSnapshot>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CommandResultWire {
    schema_version: u32,
    command: CommandId,
    run: RunId,
    command_fingerprint: IntegrityDigest,
    disposition: CommandDisposition,
    resulting_sequence: RunSequence,
    event_ids: Vec<crate::EventId>,
    result: BoundedJson,
    #[serde(default)]
    authorization: Option<AuthorityDecisionSnapshot>,
}

impl<'de> Deserialize<'de> for CommandResultDocument {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = CommandResultWire::deserialize(deserializer)?;
        if !matches!(
            wire.schema_version,
            COMMAND_RESULT_SCHEMA_VERSION_V1 | COMMAND_RESULT_SCHEMA_VERSION_V2
        ) {
            return Err(serde::de::Error::custom(format!(
                "unsupported command_result schema version {}; supported version is {}",
                wire.schema_version, COMMAND_RESULT_SCHEMA_VERSION_V2
            )));
        }
        Self::build(
            wire.schema_version,
            wire.command,
            wire.run,
            wire.command_fingerprint,
            wire.disposition,
            wire.resulting_sequence,
            wire.event_ids,
            wire.result,
            wire.authorization,
        )
        .map_err(serde::de::Error::custom)
    }
}

impl CommandResultDocument {
    /// Constructs the exact result retained by the idempotency record.
    #[allow(clippy::too_many_arguments)] // One validated durable document keeps its complete storage facts explicit.
    pub fn new(
        command: CommandId,
        run: RunId,
        command_fingerprint: IntegrityDigest,
        disposition: CommandDisposition,
        resulting_sequence: RunSequence,
        event_ids: Vec<crate::EventId>,
        result: BoundedJson,
    ) -> Result<Self, PersistenceError> {
        Self::build(
            COMMAND_RESULT_SCHEMA_VERSION_V1,
            command,
            run,
            command_fingerprint,
            disposition,
            resulting_sequence,
            event_ids,
            result,
            None,
        )
    }

    /// Constructs an authorization-bearing external command result.
    #[allow(clippy::too_many_arguments)] // One validated durable document keeps its complete storage facts explicit.
    pub fn new_authorized(
        command: CommandId,
        run: RunId,
        command_fingerprint: IntegrityDigest,
        disposition: CommandDisposition,
        resulting_sequence: RunSequence,
        event_ids: Vec<crate::EventId>,
        result: BoundedJson,
        authorization: AuthorityDecisionSnapshot,
    ) -> Result<Self, PersistenceError> {
        Self::build(
            COMMAND_RESULT_SCHEMA_VERSION_V2,
            command,
            run,
            command_fingerprint,
            disposition,
            resulting_sequence,
            event_ids,
            result,
            Some(authorization),
        )
    }

    #[allow(clippy::too_many_arguments)] // One validated durable document keeps its complete storage facts explicit.
    fn build(
        schema_version: u32,
        command: CommandId,
        run: RunId,
        command_fingerprint: IntegrityDigest,
        disposition: CommandDisposition,
        resulting_sequence: RunSequence,
        event_ids: Vec<crate::EventId>,
        result: BoundedJson,
        authorization: Option<AuthorityDecisionSnapshot>,
    ) -> Result<Self, PersistenceError> {
        if (schema_version == COMMAND_RESULT_SCHEMA_VERSION_V2) != authorization.is_some() {
            return Err(PersistenceError::InvalidDocument(
                "command-result schema v2 requires authorization and v1 forbids it".to_owned(),
            ));
        }
        if event_ids.len() > MAX_EVENTS_PER_COMMIT {
            return Err(PersistenceError::Bounds {
                location: "command_result.event_ids",
                reason: format!("at most {MAX_EVENTS_PER_COMMIT} event identities are allowed"),
            });
        }
        if matches!(disposition, CommandDisposition::Accepted) == event_ids.is_empty() {
            return Err(PersistenceError::InvalidDocument(
                "accepted command results require events and rejected results require none"
                    .to_owned(),
            ));
        }
        let mut unique = BTreeSet::new();
        if !event_ids.iter().all(|event| unique.insert(event)) {
            return Err(PersistenceError::InvalidDocument(
                "a command result cannot contain duplicate event identities".to_owned(),
            ));
        }
        Ok(Self {
            schema_version,
            command,
            run,
            command_fingerprint,
            disposition,
            resulting_sequence,
            event_ids,
            result,
            authorization,
        })
    }

    /// Document schema.
    #[must_use]
    pub const fn schema_version(&self) -> u32 {
        self.schema_version
    }

    /// Command identity.
    #[must_use]
    pub const fn command(&self) -> &CommandId {
        &self.command
    }

    /// Aggregate identity.
    #[must_use]
    pub const fn run(&self) -> &RunId {
        &self.run
    }

    /// Fingerprint that must match a redelivered receipt.
    #[must_use]
    pub const fn command_fingerprint(&self) -> &IntegrityDigest {
        &self.command_fingerprint
    }

    /// Accepted/rejected disposition.
    #[must_use]
    pub const fn disposition(&self) -> CommandDisposition {
        self.disposition
    }

    /// Authoritative journal sequence after the original result.
    #[must_use]
    pub const fn resulting_sequence(&self) -> RunSequence {
        self.resulting_sequence
    }

    /// Event identities emitted by the command, in sequence order.
    #[must_use]
    pub fn event_ids(&self) -> &[crate::EventId] {
        &self.event_ids
    }

    /// Bounded runtime-owned typed result payload.
    #[must_use]
    pub const fn result(&self) -> &BoundedJson {
        &self.result
    }

    /// Exact immutable external authorization decision, absent only for internal/v1 results.
    #[must_use]
    pub const fn authorization(&self) -> Option<&AuthorityDecisionSnapshot> {
        self.authorization.as_ref()
    }

    /// Serializes recursively key-sorted canonical compact JSON.
    pub fn to_canonical_json(&self) -> Result<Vec<u8>, PersistenceError> {
        crate::document::canonical_json_bytes(self, MAX_COMMAND_RESULT_DOCUMENT_BYTES)
    }

    /// Decodes, version-checks, and revalidates a persisted command result.
    pub fn from_json(bytes: &[u8]) -> Result<Self, PersistenceError> {
        if bytes.len() > MAX_COMMAND_RESULT_DOCUMENT_BYTES {
            return Err(PersistenceError::Bounds {
                location: "command_result.document",
                reason: format!("exceeds {MAX_COMMAND_RESULT_DOCUMENT_BYTES} bytes"),
            });
        }
        let value = crate::document::parse_json_without_duplicates(bytes)?;
        let version = value
            .get("schema_version")
            .and_then(serde_json::Value::as_u64)
            .and_then(|value| u32::try_from(value).ok())
            .ok_or_else(|| {
                PersistenceError::InvalidDocument(
                    "command result requires a numeric u32 schema_version".to_owned(),
                )
            })?;
        if !matches!(
            version,
            COMMAND_RESULT_SCHEMA_VERSION_V1 | COMMAND_RESULT_SCHEMA_VERSION_V2
        ) {
            return Err(PersistenceError::UnsupportedVersion {
                document: "command_result",
                found: version,
                supported: COMMAND_RESULT_SCHEMA_VERSION_V2,
            });
        }
        Ok(serde_json::from_value(value)?)
    }
}
