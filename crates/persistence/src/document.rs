use milkdrift_workspace::RunId;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use milkdrift_contracts::{
    CanonicalJsonError, JsonBoundKind, JsonBoundViolation, JsonLimits,
    canonical_json_bytes as encode_canonical_json,
};

use crate::{
    EventId, IntegrityDigest, PersistenceError, RunEventKind, RunSequence, TimestampMillis,
};

/// Legacy run-event envelope schema retained for exact historical reads.
pub const RUN_EVENT_SCHEMA_VERSION_V1: u32 = 1;
/// Current run-event envelope schema with controller, attributed reconciliation, and child-usage facts.
pub const RUN_EVENT_SCHEMA_VERSION_V2: u32 = 2;
/// Current run-event envelope schema with atomic controller final-entry admission facts.
pub const RUN_EVENT_SCHEMA_VERSION_V3: u32 = 3;
/// Maximum encoded event size. Larger content belongs in workspace/artifact storage.
pub const MAX_EVENT_DOCUMENT_BYTES: usize = 1_048_576;
const MAX_DOCUMENT_DEPTH: usize = 64;
const MAX_CONTAINER_ITEMS: usize = 4_096;
const MAX_STRING_BYTES: usize = 65_536;
const EVENT_CHECKSUM_DOMAIN_V1: &str = "milkdrift.run-event-envelope.v1";
const EVENT_CHECKSUM_DOMAIN_V2: &str = "milkdrift.run-event-envelope.v2";
const EVENT_CHECKSUM_DOMAIN_V3: &str = "milkdrift.run-event-envelope.v3";
const PERSISTENCE_JSON_LIMITS: JsonLimits = JsonLimits {
    maximum_depth: MAX_DOCUMENT_DEPTH,
    maximum_string_bytes: MAX_STRING_BYTES,
    maximum_key_bytes: MAX_STRING_BYTES,
    maximum_container_items: MAX_CONTAINER_ITEMS,
};

/// Explicit, checksummed append-only event envelope.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RunEventEnvelope {
    schema_version: u32,
    event_id: EventId,
    run_id: RunId,
    sequence: RunSequence,
    occurred_at: TimestampMillis,
    kind: RunEventKind,
    checksum: IntegrityDigest,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RunEventEnvelopeWire {
    schema_version: u32,
    event_id: EventId,
    run_id: RunId,
    sequence: RunSequence,
    occurred_at: TimestampMillis,
    kind: RunEventKind,
    checksum: IntegrityDigest,
}

#[derive(Serialize)]
struct EventChecksumInput<'a> {
    domain: &'static str,
    schema_version: u32,
    event_id: &'a EventId,
    run_id: &'a RunId,
    sequence: RunSequence,
    occurred_at: TimestampMillis,
    kind: &'a RunEventKind,
}

impl RunEventEnvelope {
    /// Constructs a validated envelope and calculates its canonical checksum.
    pub fn new(
        event_id: EventId,
        run_id: RunId,
        sequence: RunSequence,
        occurred_at: TimestampMillis,
        kind: RunEventKind,
    ) -> Result<Self, PersistenceError> {
        if sequence == RunSequence::ZERO {
            return Err(PersistenceError::InvalidDocument(
                "event sequence must be greater than zero".to_owned(),
            ));
        }
        kind.validate_for_run(&run_id)?;
        validate_event_schema_semantics(&kind, RUN_EVENT_SCHEMA_VERSION_V3)?;
        let checksum = calculate_event_checksum(
            RUN_EVENT_SCHEMA_VERSION_V3,
            &event_id,
            &run_id,
            sequence,
            occurred_at,
            &kind,
        )?;
        let envelope = Self {
            schema_version: RUN_EVENT_SCHEMA_VERSION_V3,
            event_id,
            run_id,
            sequence,
            occurred_at,
            kind,
            checksum,
        };
        // Enforce the encoded bound at the trusted construction boundary too.
        let _ = envelope.to_canonical_json()?;
        Ok(envelope)
    }

    /// Returns the explicit envelope schema.
    #[must_use]
    pub const fn schema_version(&self) -> u32 {
        self.schema_version
    }

    /// Returns the stable event identity.
    #[must_use]
    pub const fn event_id(&self) -> &EventId {
        &self.event_id
    }

    /// Returns the owning run aggregate.
    #[must_use]
    pub const fn run_id(&self) -> &RunId {
        &self.run_id
    }

    /// Returns the authoritative per-run event sequence.
    #[must_use]
    pub const fn sequence(&self) -> RunSequence {
        self.sequence
    }

    /// Returns the boundary-clock timestamp recorded with the fact.
    #[must_use]
    pub const fn occurred_at(&self) -> TimestampMillis {
        self.occurred_at
    }

    /// Returns the closed event fact.
    #[must_use]
    pub const fn kind(&self) -> &RunEventKind {
        &self.kind
    }

    /// Returns the canonical envelope checksum.
    #[must_use]
    pub const fn checksum(&self) -> &IntegrityDigest {
        &self.checksum
    }

    /// Encodes deterministic recursively key-sorted compact JSON.
    pub fn to_canonical_json(&self) -> Result<Vec<u8>, PersistenceError> {
        canonical_json_bytes(self, MAX_EVENT_DOCUMENT_BYTES)
    }

    /// Bounds-checks, duplicate-key-checks, version-checks, decodes, and verifies JSON.
    ///
    /// Future schemas, malformed fields, and checksum failures are returned explicitly;
    /// none are interpreted as an absent event.
    pub fn from_json(bytes: &[u8]) -> Result<Self, PersistenceError> {
        if bytes.len() > MAX_EVENT_DOCUMENT_BYTES {
            return Err(PersistenceError::Bounds {
                location: "event_document",
                reason: format!("exceeds {MAX_EVENT_DOCUMENT_BYTES} bytes"),
            });
        }
        let value = parse_json_without_duplicates(bytes)?;
        validate_value(&value, "$", 0)?;
        let version = value
            .get("schema_version")
            .and_then(Value::as_u64)
            .and_then(|value| u32::try_from(value).ok())
            .ok_or_else(|| {
                PersistenceError::InvalidDocument(
                    "run event requires a numeric u32 schema_version".to_owned(),
                )
            })?;
        if !matches!(
            version,
            RUN_EVENT_SCHEMA_VERSION_V1 | RUN_EVENT_SCHEMA_VERSION_V2 | RUN_EVENT_SCHEMA_VERSION_V3
        ) {
            return Err(PersistenceError::UnsupportedVersion {
                document: "run_event",
                found: version,
                supported: RUN_EVENT_SCHEMA_VERSION_V3,
            });
        }
        validate_event_schema_shape(&value, version)?;
        let wire: RunEventEnvelopeWire = serde_json::from_value(value)?;
        wire.kind.validate_for_run(&wire.run_id)?;
        validate_event_schema_semantics(&wire.kind, version)?;
        if wire.sequence == RunSequence::ZERO {
            return Err(PersistenceError::InvalidDocument(
                "event sequence must be greater than zero".to_owned(),
            ));
        }
        let expected = calculate_event_checksum(
            wire.schema_version,
            &wire.event_id,
            &wire.run_id,
            wire.sequence,
            wire.occurred_at,
            &wire.kind,
        )?;
        if expected != wire.checksum {
            return Err(PersistenceError::Corruption(format!(
                "event {} checksum mismatch: expected {expected}, stored {}",
                wire.event_id, wire.checksum
            )));
        }
        Ok(Self {
            schema_version: wire.schema_version,
            event_id: wire.event_id,
            run_id: wire.run_id,
            sequence: wire.sequence,
            occurred_at: wire.occurred_at,
            kind: wire.kind,
            checksum: wire.checksum,
        })
    }
}

fn calculate_event_checksum(
    schema_version: u32,
    event_id: &EventId,
    run_id: &RunId,
    sequence: RunSequence,
    occurred_at: TimestampMillis,
    kind: &RunEventKind,
) -> Result<IntegrityDigest, PersistenceError> {
    let domain = match schema_version {
        RUN_EVENT_SCHEMA_VERSION_V1 => EVENT_CHECKSUM_DOMAIN_V1,
        RUN_EVENT_SCHEMA_VERSION_V2 => EVENT_CHECKSUM_DOMAIN_V2,
        RUN_EVENT_SCHEMA_VERSION_V3 => EVENT_CHECKSUM_DOMAIN_V3,
        _ => {
            return Err(PersistenceError::UnsupportedVersion {
                document: "run_event",
                found: schema_version,
                supported: RUN_EVENT_SCHEMA_VERSION_V3,
            });
        }
    };
    let input = EventChecksumInput {
        domain,
        schema_version,
        event_id,
        run_id,
        sequence,
        occurred_at,
        kind,
    };
    let bytes = canonical_json_bytes(&input, MAX_EVENT_DOCUMENT_BYTES)?;
    Ok(IntegrityDigest::hash(&bytes))
}

fn validate_event_schema_shape(value: &Value, version: u32) -> Result<(), PersistenceError> {
    if version != RUN_EVENT_SCHEMA_VERSION_V1 {
        return Ok(());
    }
    let kind = value
        .get("kind")
        .and_then(Value::as_object)
        .ok_or_else(|| {
            PersistenceError::InvalidDocument("run event kind must be an object".to_owned())
        })?;
    let event_type = kind.get("type").and_then(Value::as_str).ok_or_else(|| {
        PersistenceError::InvalidDocument("run event kind requires a string type".to_owned())
    })?;
    let contains_v2_field = match event_type {
        "controller_assessment_recorded" => true,
        "subworkflow_terminal" => kind.contains_key("usage"),
        "revision_adoption_requested" => kind.contains_key("requested_by"),
        "capability_resolved" | "capability_resolution_decision_recorded" => kind
            .get("snapshot")
            .and_then(Value::as_object)
            .is_some_and(|snapshot| snapshot.contains_key("category")),
        _ => false,
    };
    if contains_v2_field {
        return Err(PersistenceError::InvalidDocument(
            "run-event schema v1 cannot contain v2-only semantic fields".to_owned(),
        ));
    }
    Ok(())
}

fn validate_event_schema_semantics(
    kind: &RunEventKind,
    version: u32,
) -> Result<(), PersistenceError> {
    match (version, kind) {
        (RUN_EVENT_SCHEMA_VERSION_V1, RunEventKind::ControllerAssessmentRecorded { .. }) => {
            Err(PersistenceError::InvalidDocument(
                "controller assessment requires run-event schema v2".to_owned(),
            ))
        }
        (RUN_EVENT_SCHEMA_VERSION_V1, RunEventKind::SubworkflowTerminal { usage, .. })
            if !usage.is_empty() =>
        {
            Err(PersistenceError::InvalidDocument(
                "child resource usage requires run-event schema v2".to_owned(),
            ))
        }
        (
            RUN_EVENT_SCHEMA_VERSION_V1,
            RunEventKind::RevisionAdoptionRequested {
                requested_by: Some(_),
                ..
            },
        ) => Err(PersistenceError::InvalidDocument(
            "attributed revision adoption requires run-event schema v2".to_owned(),
        )),
        (
            RUN_EVENT_SCHEMA_VERSION_V1,
            RunEventKind::CapabilityResolved { snapshot, .. }
            | RunEventKind::CapabilityResolutionDecisionRecorded { snapshot, .. },
        ) if snapshot.category().is_some() => Err(PersistenceError::InvalidDocument(
            "category-bound capability resolution requires run-event schema v2".to_owned(),
        )),
        (
            RUN_EVENT_SCHEMA_VERSION_V1 | RUN_EVENT_SCHEMA_VERSION_V2,
            RunEventKind::SubworkflowTerminal {
                cost_micros, usage, ..
            },
        ) if cost_micros != &usage.cost_micros => Err(PersistenceError::InvalidDocument(
            "child cost and complete resource usage ledgers disagree".to_owned(),
        )),
        (
            RUN_EVENT_SCHEMA_VERSION_V2,
            RunEventKind::RevisionAdoptionRequested {
                requested_by: None, ..
            },
        ) => Err(PersistenceError::InvalidDocument(
            "run-event schema v2 revision adoption requires actor attribution".to_owned(),
        )),
        (
            RUN_EVENT_SCHEMA_VERSION_V2,
            RunEventKind::CapabilityResolved { snapshot, .. }
            | RunEventKind::CapabilityResolutionDecisionRecorded { snapshot, .. },
        ) if snapshot.category().is_none() => Err(PersistenceError::InvalidDocument(
            "run-event schema v2 capability resolution requires a descriptor category".to_owned(),
        )),
        (
            RUN_EVENT_SCHEMA_VERSION_V2,
            RunEventKind::CapabilityAdapterEntryDecisionRecorded {
                controller_admission,
                ..
            },
        ) if !matches!(
            controller_admission,
            crate::ControllerAdmissionOutcome::NotControlled
        ) =>
        {
            Err(PersistenceError::InvalidDocument(
                "controller final-entry admission requires run-event schema v3".to_owned(),
            ))
        }
        (
            RUN_EVENT_SCHEMA_VERSION_V2,
            RunEventKind::ControllerAssessmentRecorded {
                account_declaration: Some(_),
                ..
            },
        ) => Err(PersistenceError::InvalidDocument(
            "controller account declaration requires run-event schema v3".to_owned(),
        )),
        (
            RUN_EVENT_SCHEMA_VERSION_V3,
            RunEventKind::ControllerAssessmentRecorded {
                account_declaration: None,
                ..
            },
        ) => Err(PersistenceError::InvalidDocument(
            "run-event schema v3 controller assessment requires an account declaration".to_owned(),
        )),
        (
            RUN_EVENT_SCHEMA_VERSION_V3,
            RunEventKind::RevisionAdoptionRequested {
                requested_by: None, ..
            },
        ) => Err(PersistenceError::InvalidDocument(
            "run-event schema v3 revision adoption requires actor attribution".to_owned(),
        )),
        (
            RUN_EVENT_SCHEMA_VERSION_V3,
            RunEventKind::CapabilityResolved { snapshot, .. }
            | RunEventKind::CapabilityResolutionDecisionRecorded { snapshot, .. },
        ) if snapshot.category().is_none() => Err(PersistenceError::InvalidDocument(
            "run-event schema v3 capability resolution requires a descriptor category".to_owned(),
        )),
        (
            RUN_EVENT_SCHEMA_VERSION_V3,
            RunEventKind::SubworkflowTerminal {
                cost_micros, usage, ..
            },
        ) if cost_micros != &usage.cost_micros => Err(PersistenceError::InvalidDocument(
            "child cost and complete resource usage ledgers disagree".to_owned(),
        )),
        _ => Ok(()),
    }
}

pub(crate) fn canonical_json_bytes<T: Serialize>(
    value: &T,
    maximum_bytes: usize,
) -> Result<Vec<u8>, PersistenceError> {
    let bytes =
        encode_canonical_json(value, PERSISTENCE_JSON_LIMITS).map_err(|error| match error {
            CanonicalJsonError::Json(error) => PersistenceError::Json(error),
            CanonicalJsonError::Bounds(violation) => persistence_bound(violation),
        })?;
    if bytes.len() > maximum_bytes {
        return Err(PersistenceError::Bounds {
            location: "document",
            reason: format!("canonical JSON exceeds {maximum_bytes} bytes"),
        });
    }
    Ok(bytes)
}

fn validate_value(value: &Value, location: &str, depth: usize) -> Result<(), PersistenceError> {
    debug_assert_eq!(location, "$", "persistence validation starts at the root");
    debug_assert_eq!(depth, 0, "persistence validation starts at depth zero");
    milkdrift_contracts::validate_json_value(value, PERSISTENCE_JSON_LIMITS)
        .map_err(persistence_bound)
}

pub(crate) fn parse_json_without_duplicates(bytes: &[u8]) -> Result<Value, PersistenceError> {
    milkdrift_contracts::parse_json_without_duplicates(bytes).map_err(PersistenceError::Json)
}

fn persistence_bound(violation: JsonBoundViolation) -> PersistenceError {
    let (location, reason) = match violation.kind() {
        JsonBoundKind::Depth => (
            "document.depth",
            format!("{} exceeds depth {}", violation.path(), violation.maximum()),
        ),
        JsonBoundKind::String => (
            "document.string",
            format!("{} exceeds {} bytes", violation.path(), violation.maximum()),
        ),
        JsonBoundKind::Key => (
            "document.key",
            format!(
                "key below {} exceeds {} bytes",
                violation.path(),
                violation.maximum()
            ),
        ),
        JsonBoundKind::Array => (
            "document.array",
            format!(
                "{} exceeds {} entries",
                violation.path(),
                violation.maximum()
            ),
        ),
        JsonBoundKind::Object => (
            "document.object",
            format!(
                "{} exceeds {} entries",
                violation.path(),
                violation.maximum()
            ),
        ),
    };
    PersistenceError::Bounds { location, reason }
}
