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

/// Retains command identity and intent so a lost reply can be recovered without another append.
///
/// The runtime supplies a complete audit document and the intent whose meaning must
/// stay unchanged across retries. Storage retains both and compares the fingerprint
/// for the same `(run, command)` before checking a new delivery's expected sequence.
///
/// Use [`Self::new`] when every document byte belongs to the intent. Runtime callers
/// use [`Self::new_idempotent`] to separate delivery metadata from intent. Both check
/// canonical JSON; the runtime owns command meaning and authority. Include the receipt
/// in an [`AtomicRunCommitRequest`](crate::AtomicRunCommitRequest).
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
    /// Uses the same canonical JSON bytes for audit and intent.
    ///
    /// Any change to those bytes changes the fingerprint. The separate
    /// `expected_sequence` and `submitted_at` arguments are retained metadata; they
    /// affect the fingerprint only if the caller also includes them in the document.
    /// See [`Self::new_idempotent`] for byte requirements and errors.
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

    /// Separates the complete audit document from the intent used to identify retries.
    ///
    /// The fingerprint binds `command`, `run`, `actor`, and `canonical_intent`, with
    /// length framing and a domain separator. It excludes `expected_sequence`,
    /// `submitted_at`, and `canonical_document`. The runtime may therefore replan the
    /// same intent at a newer sequence after a conflict. If storage already saved a
    /// result, it returns that first result and keeps the original audit document.
    ///
    /// The caller must include every fact that changes command meaning in the intent,
    /// including any authority claim. Persistence cannot infer those facts or check
    /// that the two documents agree. This constructor does not relax an external
    /// protocol's complete-request idempotency rules.
    ///
    /// # Errors
    ///
    /// Each document must contain `1..=MAX_COMMAND_DOCUMENT_BYTES` bytes of compact,
    /// recursively key-sorted JSON. Empty/oversized inputs return
    /// [`PersistenceError::Bounds`]; malformed JSON returns [`PersistenceError::Json`].
    /// Noncanonical bytes, including whitespace or duplicate keys, are refused, as are
    /// documents that exceed the persistence JSON structure limits.
    ///
    /// # Example
    ///
    /// This storage-facing example uses a small illustrative intent, not a runtime
    /// command wire document. The runtime owns production command encoding.
    ///
    /// ```
    /// use milkdrift_authority::ActorRef;
    /// use milkdrift_persistence::{CommandId, CommandReceipt, RunSequence, TimestampMillis};
    /// use milkdrift_workspace::RunId;
    ///
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let command = CommandId::new("command:pause")?;
    /// let run = RunId::new("run:example")?;
    /// let actor = ActorRef::new("human:operator")?;
    /// let first = CommandReceipt::new_idempotent(
    ///     command.clone(), run.clone(), actor.clone(),
    ///     RunSequence::new(4), TimestampMillis::new(100),
    ///     br#"{"delivery":100,"type":"pause"}"#.to_vec(),
    ///     br#"{"type":"pause"}"#.to_vec(),
    /// )?;
    /// let redelivery = CommandReceipt::new_idempotent(
    ///     command.clone(), run.clone(), actor.clone(),
    ///     RunSequence::new(7), TimestampMillis::new(200),
    ///     br#"{"delivery":200,"type":"pause"}"#.to_vec(),
    ///     br#"{"type":"pause"}"#.to_vec(),
    /// )?;
    /// assert_eq!(first.fingerprint(), redelivery.fingerprint());
    /// assert_ne!(first.canonical_document(), redelivery.canonical_document());
    ///
    /// let changed = CommandReceipt::new(
    ///     command, run, actor, RunSequence::new(7), TimestampMillis::new(200),
    ///     br#"{"type":"resume"}"#.to_vec(),
    /// )?;
    /// assert_ne!(first.fingerprint(), changed.fingerprint());
    /// # Ok(())
    /// # }
    /// ```
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

    /// Journal head the runtime planned against; checked only for a new command.
    #[must_use]
    pub const fn expected_sequence(&self) -> RunSequence {
        self.expected_sequence
    }

    /// Caller-supplied receipt time in milliseconds since the Unix epoch.
    /// This type stores the observation; it does not read or advance a clock.
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
    /// Storage uses these bytes to verify the fingerprint without interpreting runtime fields.
    #[must_use]
    pub fn canonical_intent(&self) -> &[u8] {
        &self.canonical_intent
    }

    /// Digest binding command, run, actor, and canonical intent for replay comparison.
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

/// Response that storage retains with a command receipt and returns on redelivery.
///
/// The runtime supplies the disposition, ordered event IDs, resulting sequence, and
/// bounded result payload. [`crate::AtomicRunCommitRequest::new`] checks them against
/// the receipt and actual proposed append. Constructing or decoding this value does
/// not prove that any command has committed; retrieve that evidence through
/// [`crate::RunJournal::command_result`] or [`crate::RunJournal::commit_command`].
///
/// External results carry the original authority decision. Replay returns that
/// decision and payload without reevaluating authority. A durable rejection has no
/// events; it is still a saved result, unlike a storage error.
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
    /// Constructs a version 1 result for an internal command, without authorization.
    ///
    /// Use [`Self::new_authorized`] for an external command. `event_ids` must be in
    /// append order and `resulting_sequence` must name the resulting journal head;
    /// the enclosing commit checks that correspondence. `result` is the runtime's
    /// response payload, not event or artifact content.
    ///
    /// # Errors
    ///
    /// Returns [`PersistenceError::Bounds`] above [`MAX_EVENTS_PER_COMMIT`] event IDs.
    /// Duplicate IDs, accepted results without events, and rejected results with
    /// events return [`PersistenceError::InvalidDocument`].
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

    /// Constructs a version 2 result retaining the external authority decision.
    ///
    /// The caller evaluates authority before constructing this record and supplies
    /// the exact decision used for this command, including a denial when applicable.
    /// This method checks document shape; it does not authorize execution or establish
    /// that the decision belongs to the supplied command. See [`Self::new`] for
    /// event/sequence inputs and their errors. Serialization retains the decision so
    /// redelivery can return it even after current authority changes.
    #[allow(clippy::too_many_arguments)] // External results bind exact command identity and disposition to event identities, sequence, payload, and authorization.
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

    #[allow(clippy::too_many_arguments)] // One constructor enforces schema/authorization consistency and accepted/rejected event invariants for both readers.
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

    /// Journal head after the original command, which may precede today's head.
    /// A rejected result retains the receipt's expected sequence and adds no events.
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

    /// Encodes compact, recursively key-sorted JSON for storage or exact comparison.
    ///
    /// # Errors
    ///
    /// Returns an error if encoding exceeds [`MAX_COMMAND_RESULT_DOCUMENT_BYTES`] or
    /// the persistence JSON structure limits, or if serialization fails.
    pub fn to_canonical_json(&self) -> Result<Vec<u8>, PersistenceError> {
        crate::document::canonical_json_bytes(self, MAX_COMMAND_RESULT_DOCUMENT_BYTES)
    }

    /// Reads a version 1 internal or version 2 authorization-bearing result.
    ///
    /// Version 1 must omit authorization; version 2 must contain it. The reader
    /// accepts noncanonical formatting and revalidates shape, nested documents, and
    /// event-ID constraints. Use [`Self::to_canonical_json`] for canonical bytes;
    /// storage separately verifies the result against its receipt and journal.
    ///
    /// # Errors
    ///
    /// Refuses documents above [`MAX_COMMAND_RESULT_DOCUMENT_BYTES`], duplicate keys,
    /// malformed JSON, unknown fields, and inconsistent result contents. Nested fields
    /// enforce their own readers' bounds; this does not run the writer's generic JSON
    /// structure check.
    /// Unsupported numeric versions return [`PersistenceError::UnsupportedVersion`];
    /// malformed fields or nested documents also fail rather than being ignored.
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
