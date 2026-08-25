use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

use milkdrift_contracts::{
    CanonicalJsonError, JsonBoundKind, JsonBoundViolation, JsonLimits,
    canonical_json_bytes as encode_canonical_json,
};

use crate::{
    AuthorRef, BLUEPRINT_SCHEMA_VERSION_V1, BlueprintId, BlueprintMetadata, BlueprintRevision,
    ContentDigest, Edge, EdgeId, MutationError, Node, NodeFingerprint, NodeId, RevisionId,
    SemanticBlueprint, ValidationError, WorkflowId, WorkflowInterface,
};

const MAX_BLUEPRINT_DOCUMENT_BYTES: usize = 4_194_304;
const MAX_BLUEPRINT_DOCUMENT_DEPTH: usize = 64;
const MAX_JSON_STRING_BYTES: usize = 65_536;
const MAX_JSON_CONTAINER_ITEMS: usize = 8_192;
const BLUEPRINT_JSON_LIMITS: JsonLimits = JsonLimits {
    maximum_depth: MAX_BLUEPRINT_DOCUMENT_DEPTH,
    maximum_string_bytes: MAX_JSON_STRING_BYTES,
    maximum_key_bytes: 256,
    maximum_container_items: MAX_JSON_CONTAINER_ITEMS,
};

/// Error returned while reading or writing a portable blueprint revision document.
#[derive(Debug, Error)]
pub enum DocumentError {
    /// JSON syntax or data shape was invalid.
    #[error("invalid blueprint JSON: {0}")]
    Json(#[from] serde_json::Error),
    /// Document exceeded a hostile-input bound.
    #[error("blueprint document bound exceeded at {location}: {reason}")]
    Bounds {
        /// JSON-like location.
        location: String,
        /// Violated limit.
        reason: String,
    },
    /// A future or otherwise unsupported schema was supplied.
    #[error("unsupported blueprint schema version {found}; supported version is {supported}")]
    UnsupportedVersion {
        /// Version found on input.
        found: u32,
        /// Version implemented here.
        supported: u32,
    },
    /// Graph semantic validation failed.
    #[error(transparent)]
    Validation(#[from] ValidationError),
    /// Digest, revision identity, ancestry, or bounded revision metadata was invalid.
    #[error("invalid blueprint revision: {0}")]
    Integrity(String),
}

impl From<MutationError> for DocumentError {
    fn from(error: MutationError) -> Self {
        match error {
            MutationError::Validation(error) => Self::Validation(error),
            other => Self::Integrity(other.to_string()),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
struct RevisionWire {
    id: RevisionId,
    sequence: u64,
    content_digest: ContentDigest,
    parents: Vec<RevisionId>,
    author: AuthorRef,
    reason: String,
    semantic: SemanticWire,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct SemanticWire {
    workflow: WorkflowId,
    blueprint: BlueprintId,
    metadata: BlueprintMetadata,
    interface: WorkflowInterface,
    nodes: BTreeMap<NodeId, Node>,
    edges: BTreeMap<EdgeId, Edge>,
}

impl From<&SemanticBlueprint> for SemanticWire {
    fn from(semantic: &SemanticBlueprint) -> Self {
        Self {
            workflow: semantic.workflow().clone(),
            blueprint: semantic.blueprint().clone(),
            metadata: semantic.metadata().clone(),
            interface: semantic.interface().clone(),
            nodes: semantic.nodes().clone(),
            edges: semantic.edges().clone(),
        }
    }
}

impl RevisionWire {
    fn from_revision(revision: &BlueprintRevision) -> Self {
        Self {
            id: revision.id().clone(),
            sequence: revision.sequence(),
            content_digest: revision.content_digest().clone(),
            parents: revision.parents().to_vec(),
            author: revision.author().clone(),
            reason: revision.reason().to_owned(),
            semantic: SemanticWire::from(revision.semantic()),
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RevisionReadWire {
    id: RevisionId,
    sequence: u64,
    content_digest: ContentDigest,
    parents: Vec<RevisionId>,
    author: AuthorRef,
    reason: String,
    semantic: SemanticWire,
}

/// Canonical schema-v1 compatibility envelope for an immutable blueprint revision.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct BlueprintRevisionDocument {
    schema_version: u32,
    revision: RevisionWire,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BlueprintDocumentWire {
    schema_version: u32,
    revision: RevisionReadWire,
}

impl BlueprintRevisionDocument {
    /// Wraps an immutable revision in the current version envelope.
    #[must_use]
    pub fn new(revision: &BlueprintRevision) -> Self {
        Self {
            schema_version: BLUEPRINT_SCHEMA_VERSION_V1,
            revision: RevisionWire::from_revision(revision),
        }
    }

    /// Current envelope version.
    #[must_use]
    pub const fn schema_version(&self) -> u32 {
        self.schema_version
    }

    /// Serializes as recursively key-sorted compact JSON.
    pub fn to_canonical_json(&self) -> Result<Vec<u8>, DocumentError> {
        canonical_value_bytes(self)
    }

    /// Reads, bounds-checks, version-checks, validates, and integrity-checks a revision.
    pub fn from_json(bytes: &[u8]) -> Result<(Self, BlueprintRevision), DocumentError> {
        if bytes.len() > MAX_BLUEPRINT_DOCUMENT_BYTES {
            return Err(DocumentError::Bounds {
                location: "$".to_owned(),
                reason: format!("document exceeds {MAX_BLUEPRINT_DOCUMENT_BYTES} bytes"),
            });
        }
        let value = milkdrift_contracts::parse_json_without_duplicates(bytes)?;
        validate_value(&value, "$", 0)?;
        let version = value
            .get("schema_version")
            .and_then(Value::as_u64)
            .and_then(|value| u32::try_from(value).ok())
            .ok_or_else(|| DocumentError::Integrity("missing numeric schema_version".to_owned()))?;
        if version != BLUEPRINT_SCHEMA_VERSION_V1 {
            return Err(DocumentError::UnsupportedVersion {
                found: version,
                supported: BLUEPRINT_SCHEMA_VERSION_V1,
            });
        }
        let wire: BlueprintDocumentWire = serde_json::from_value(value)?;
        if wire.schema_version != BLUEPRINT_SCHEMA_VERSION_V1 {
            return Err(DocumentError::UnsupportedVersion {
                found: wire.schema_version,
                supported: BLUEPRINT_SCHEMA_VERSION_V1,
            });
        }
        let semantic = SemanticBlueprint::from_parts(
            wire.revision.semantic.workflow,
            wire.revision.semantic.blueprint,
            wire.revision.semantic.metadata,
            wire.revision.semantic.interface,
            wire.revision.semantic.nodes,
            wire.revision.semantic.edges,
        );
        let revision = BlueprintRevision::from_verified_parts(
            wire.revision.id,
            wire.revision.sequence,
            wire.revision.content_digest,
            wire.revision.parents,
            wire.revision.author,
            wire.revision.reason,
            semantic,
        )?;
        Ok((Self::new(&revision), revision))
    }
}

/// Returns deterministic canonical JSON for validated semantic blueprint content.
pub fn canonical_blueprint_json(semantic: &SemanticBlueprint) -> Result<Vec<u8>, DocumentError> {
    canonical_value_bytes(semantic)
}

/// Calculates the schema-v1 domain-separated fingerprint of one immutable node definition.
pub fn node_configuration_fingerprint(node: &Node) -> Result<NodeFingerprint, DocumentError> {
    let bytes = canonical_value_bytes(node)?;
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"milkdrift.blueprint.node-configuration.v1\0");
    hasher.update(&bytes);
    Ok(NodeFingerprint::from_hash(hasher.finalize()))
}

/// Calculates a schema-v1 fingerprint of every dependency incident to one node.
///
/// The fingerprint is independent of map insertion order and deliberately excludes
/// the node configuration, allowing reconciliation to classify dependency-only edits.
pub fn node_dependency_fingerprint(
    semantic: &SemanticBlueprint,
    node: &NodeId,
) -> Result<NodeFingerprint, DocumentError> {
    let incident: Vec<_> = semantic
        .edges()
        .values()
        .filter(|edge| edge.source_node() == node || edge.target_node() == node)
        .collect();
    let bytes = canonical_value_bytes(&(node, incident))?;
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"milkdrift.blueprint.node-dependencies.v1\0");
    hasher.update(&bytes);
    Ok(NodeFingerprint::from_hash(hasher.finalize()))
}

pub(crate) fn canonical_value_bytes<T: Serialize>(value: &T) -> Result<Vec<u8>, DocumentError> {
    let bytes =
        encode_canonical_json(value, BLUEPRINT_JSON_LIMITS).map_err(|error| match error {
            CanonicalJsonError::Json(error) => DocumentError::Json(error),
            CanonicalJsonError::Bounds(violation) => blueprint_bound(violation),
        })?;
    if bytes.len() > MAX_BLUEPRINT_DOCUMENT_BYTES {
        return Err(DocumentError::Bounds {
            location: "$".to_owned(),
            reason: format!("document exceeds {MAX_BLUEPRINT_DOCUMENT_BYTES} bytes"),
        });
    }
    Ok(bytes)
}

fn validate_value(value: &Value, location: &str, depth: usize) -> Result<(), DocumentError> {
    debug_assert_eq!(location, "$", "blueprint validation starts at the root");
    debug_assert_eq!(depth, 0, "blueprint validation starts at depth zero");
    milkdrift_contracts::validate_json_value(value, BLUEPRINT_JSON_LIMITS).map_err(blueprint_bound)
}

fn blueprint_bound(violation: JsonBoundViolation) -> DocumentError {
    let reason = match violation.kind() {
        JsonBoundKind::Depth => format!("nesting exceeds depth {}", violation.maximum()),
        JsonBoundKind::String => format!("string exceeds {} bytes", violation.maximum()),
        JsonBoundKind::Key => format!("object key exceeds {} bytes", violation.maximum()),
        JsonBoundKind::Array => format!("array exceeds {} items", violation.maximum()),
        JsonBoundKind::Object => format!("object exceeds {} entries", violation.maximum()),
    };
    DocumentError::Bounds {
        location: violation.path().to_owned(),
        reason,
    }
}
