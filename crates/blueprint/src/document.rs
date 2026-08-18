use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

use crate::{
    AuthorRef, BLUEPRINT_SCHEMA_VERSION_V1, BlueprintId, BlueprintMetadata, BlueprintRevision,
    ContentDigest, Edge, EdgeId, MutationError, Node, NodeId, RevisionId, SemanticBlueprint,
    ValidationError, WorkflowId, WorkflowInterface,
};

const MAX_BLUEPRINT_DOCUMENT_BYTES: usize = 4_194_304;
const MAX_BLUEPRINT_DOCUMENT_DEPTH: usize = 64;
const MAX_JSON_STRING_BYTES: usize = 65_536;
const MAX_JSON_CONTAINER_ITEMS: usize = 8_192;

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
        let value: Value = serde_json::from_slice(bytes)?;
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

pub(crate) fn canonical_value_bytes<T: Serialize>(value: &T) -> Result<Vec<u8>, DocumentError> {
    let mut value = serde_json::to_value(value)?;
    validate_value(&value, "$", 0)?;
    sort_value(&mut value);
    let bytes = serde_json::to_vec(&value)?;
    if bytes.len() > MAX_BLUEPRINT_DOCUMENT_BYTES {
        return Err(DocumentError::Bounds {
            location: "$".to_owned(),
            reason: format!("document exceeds {MAX_BLUEPRINT_DOCUMENT_BYTES} bytes"),
        });
    }
    Ok(bytes)
}

fn sort_value(value: &mut Value) {
    match value {
        Value::Object(map) => {
            for child in map.values_mut() {
                sort_value(child);
            }
            let previous = std::mem::take(map);
            let mut entries: Vec<_> = previous.into_iter().collect();
            entries.sort_by(|left, right| left.0.cmp(&right.0));
            map.extend(entries);
        }
        Value::Array(values) => {
            for child in values {
                sort_value(child);
            }
        }
        _ => {}
    }
}

fn validate_value(value: &Value, location: &str, depth: usize) -> Result<(), DocumentError> {
    if depth > MAX_BLUEPRINT_DOCUMENT_DEPTH {
        return Err(DocumentError::Bounds {
            location: location.to_owned(),
            reason: format!("nesting exceeds depth {MAX_BLUEPRINT_DOCUMENT_DEPTH}"),
        });
    }
    match value {
        Value::String(text) if text.len() > MAX_JSON_STRING_BYTES => Err(DocumentError::Bounds {
            location: location.to_owned(),
            reason: format!("string exceeds {MAX_JSON_STRING_BYTES} bytes"),
        }),
        Value::Array(values) => {
            if values.len() > MAX_JSON_CONTAINER_ITEMS {
                return Err(DocumentError::Bounds {
                    location: location.to_owned(),
                    reason: format!("array exceeds {MAX_JSON_CONTAINER_ITEMS} items"),
                });
            }
            for (index, child) in values.iter().enumerate() {
                validate_value(child, &format!("{location}[{index}]"), depth + 1)?;
            }
            Ok(())
        }
        Value::Object(map) => {
            if map.len() > MAX_JSON_CONTAINER_ITEMS {
                return Err(DocumentError::Bounds {
                    location: location.to_owned(),
                    reason: format!("object exceeds {MAX_JSON_CONTAINER_ITEMS} entries"),
                });
            }
            for (key, child) in map {
                if key.len() > 256 {
                    return Err(DocumentError::Bounds {
                        location: location.to_owned(),
                        reason: "object key exceeds 256 bytes".to_owned(),
                    });
                }
                validate_value(child, &format!("{location}.{key}"), depth + 1)?;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}
