use std::{collections::BTreeSet, fmt, str::FromStr};

use milkdrift_capability::InvocationId;
use serde::{Deserialize, Serialize, Serializer};

use crate::{ArtifactId, CausalId, RunId, ValueKey, WorkspaceError, WorkspaceValueReference};

const BLAKE3_DIGEST_BYTES: usize = 32;
const BLAKE3_HEX_BYTES: usize = BLAKE3_DIGEST_BYTES * 2;
/// Maximum bytes in one canonical artifact media type.
pub const MAX_MEDIA_TYPE_BYTES: usize = 255;
const MAX_CAUSAL_REFERENCES: usize = 128;

/// Canonical 256-bit BLAKE3 content digest.
///
/// Its wire representation is exactly 64 lowercase hexadecimal characters.
/// Uppercase and algorithm-prefixed spellings are rejected so one digest has one
/// stable serialized form.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ContentDigest([u8; BLAKE3_DIGEST_BYTES]);

impl ContentDigest {
    /// Computes the BLAKE3 digest of complete content bytes.
    #[must_use]
    pub fn for_bytes(bytes: &[u8]) -> Self {
        Self(*blake3::hash(bytes).as_bytes())
    }

    /// Parses the canonical lowercase hexadecimal representation.
    pub fn from_hex(value: &str) -> Result<Self, WorkspaceError> {
        if value.len() != BLAKE3_HEX_BYTES {
            return Err(WorkspaceError::InvalidDigest(format!(
                "expected {BLAKE3_HEX_BYTES} lowercase hexadecimal characters"
            )));
        }
        if !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(WorkspaceError::InvalidDigest(
                "digest must use lowercase hexadecimal characters only".to_owned(),
            ));
        }

        let mut bytes = [0_u8; BLAKE3_DIGEST_BYTES];
        for (index, pair) in value.as_bytes().chunks_exact(2).enumerate() {
            bytes[index] = (hex_nibble(pair[0]) << 4) | hex_nibble(pair[1]);
        }
        Ok(Self(bytes))
    }

    /// Returns the raw 32 digest bytes without exposing a hashing-library type.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; BLAKE3_DIGEST_BYTES] {
        &self.0
    }

    /// Returns the canonical lowercase hexadecimal representation.
    #[must_use]
    pub fn to_hex(self) -> String {
        blake3::Hash::from_bytes(self.0).to_hex().to_string()
    }
}

const fn hex_nibble(byte: u8) -> u8 {
    match byte {
        b'0'..=b'9' => byte - b'0',
        b'a'..=b'f' => byte - b'a' + 10,
        _ => 0,
    }
}

impl fmt::Display for ContentDigest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.to_hex())
    }
}

impl FromStr for ContentDigest {
    type Err = WorkspaceError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::from_hex(value)
    }
}

impl Serialize for ContentDigest {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_hex())
    }
}

milkdrift_contracts::deserialize_via!(ContentDigest, String, |value| Self::from_hex(&value));

/// Validated canonical artifact media type, without parameters.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct MediaType(String);

impl MediaType {
    /// Parses and canonicalizes an RFC-token-style `type/subtype` value.
    ///
    /// Media-type parameters belong in the content's schema metadata rather than
    /// this identity. ASCII case is normalized to lowercase.
    pub fn new(value: impl Into<String>) -> Result<Self, WorkspaceError> {
        let value = value.into();
        if value.is_empty() || value.len() > MAX_MEDIA_TYPE_BYTES || !value.is_ascii() {
            return Err(WorkspaceError::InvalidMediaType(format!(
                "must contain 1 to {MAX_MEDIA_TYPE_BYTES} ASCII bytes"
            )));
        }
        let Some((top_level, subtype)) = value.split_once('/') else {
            return Err(WorkspaceError::InvalidMediaType(
                "must have the form 'type/subtype'".to_owned(),
            ));
        };
        if top_level.is_empty()
            || subtype.is_empty()
            || subtype.contains('/')
            || !top_level.bytes().all(is_media_token_byte)
            || !subtype.bytes().all(is_media_token_byte)
            || top_level.contains('*')
            || subtype.contains('*')
        {
            return Err(WorkspaceError::InvalidMediaType(
                "type and subtype must be non-wildcard RFC token text".to_owned(),
            ));
        }
        Ok(Self(value.to_ascii_lowercase()))
    }

    /// Returns the canonical `type/subtype` text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

const fn is_media_token_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric()
        || matches!(
            byte,
            b'!' | b'#'
                | b'$'
                | b'%'
                | b'&'
                | b'\''
                | b'*'
                | b'+'
                | b'-'
                | b'.'
                | b'^'
                | b'_'
                | b'`'
                | b'|'
                | b'~'
        )
}

impl fmt::Display for MediaType {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl FromStr for MediaType {
    type Err = WorkspaceError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::new(value)
    }
}

impl Serialize for MediaType {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.0)
    }
}

milkdrift_contracts::deserialize_via!(MediaType, String, |value| Self::new(value));

/// Immutable reference to separately stored content-addressed artifact bytes.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ArtifactReference {
    artifact: ArtifactId,
    digest: ContentDigest,
    media_type: MediaType,
    size_bytes: u64,
}

impl ArtifactReference {
    /// Constructs an artifact reference containing all content-verification facts.
    #[must_use]
    pub const fn new(
        artifact: ArtifactId,
        digest: ContentDigest,
        media_type: MediaType,
        size_bytes: u64,
    ) -> Self {
        Self {
            artifact,
            digest,
            media_type,
            size_bytes,
        }
    }

    /// Returns the stable logical artifact identity.
    #[must_use]
    pub const fn artifact(&self) -> &ArtifactId {
        &self.artifact
    }

    /// Returns the exact content digest.
    #[must_use]
    pub const fn digest(&self) -> ContentDigest {
        self.digest
    }

    /// Returns the validated media type.
    #[must_use]
    pub const fn media_type(&self) -> &MediaType {
        &self.media_type
    }

    /// Returns the exact byte size.
    #[must_use]
    pub const fn size_bytes(&self) -> u64 {
        self.size_bytes
    }

    /// Verifies both the exact size and BLAKE3 digest of complete bytes.
    #[must_use]
    pub fn verifies(&self, bytes: &[u8]) -> bool {
        u64::try_from(bytes.len()).is_ok_and(|size| size == self.size_bytes)
            && ContentDigest::for_bytes(bytes) == self.digest
    }
}

/// Earliest Unix timestamp at which a retention floor expires.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct RetentionDeadline(u64);

impl RetentionDeadline {
    /// Constructs a non-zero Unix timestamp in milliseconds.
    pub fn from_unix_millis(value: u64) -> Result<Self, WorkspaceError> {
        if value == 0 {
            return Err(WorkspaceError::InvalidArtifact(
                "a retention deadline must be greater than zero".to_owned(),
            ));
        }
        Ok(Self(value))
    }

    /// Returns the Unix timestamp in milliseconds.
    #[must_use]
    pub const fn as_unix_millis(self) -> u64 {
        self.0
    }
}

milkdrift_contracts::deserialize_via!(RetentionDeadline, u64, |value| Self::from_unix_millis(
    value
));

/// Sensitivity classification controlling default artifact export.
#[derive(
    Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd, Deserialize, Serialize,
)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactSensitivity {
    /// Sensitive content; access and export require explicit authorization.
    #[default]
    Restricted,
    /// Internal content; export still requires explicit authorization.
    Internal,
    /// Content explicitly classified as safe for unauthenticated export policy.
    Public,
}

impl ArtifactSensitivity {
    /// Returns whether policy may export content without separate authorization.
    #[must_use]
    pub const fn permits_unauthorized_export(self) -> bool {
        matches!(self, Self::Public)
    }
}

/// Minimum retention contract for content bytes.
///
/// Retention never authorizes deletion while durable history still references the
/// artifact; adapters must satisfy both this floor and referential integrity.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Deserialize, Serialize)]
#[serde(rename_all = "snake_case", tag = "type", deny_unknown_fields)]
pub enum ArtifactRetention {
    /// Keep content for as long as any durable record references it.
    WhileReferenced,
    /// Keep content at least until the supplied wall-clock fact.
    Until {
        /// Inclusive minimum retention deadline.
        deadline: RetentionDeadline,
    },
    /// Keep content indefinitely unless a future authorized policy supersedes it.
    Indefinite,
}

/// Exact input or producer fact in an artifact's causal history.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Deserialize, Serialize)]
#[serde(rename_all = "snake_case", tag = "type", deny_unknown_fields)]
pub enum CausalReference {
    /// One named input pinned when a run was created.
    RunInput {
        /// Owning run.
        run: RunId,
        /// Stable input key.
        key: ValueKey,
    },
    /// One exact immutable workspace value.
    WorkspaceValue {
        /// Exact scope, key, and version.
        reference: WorkspaceValueReference,
    },
    /// One exact content-addressed artifact.
    Artifact {
        /// Complete immutable artifact reference.
        reference: ArtifactReference,
    },
    /// One exact executor invocation.
    Invocation {
        /// Provider-neutral invocation identity.
        invocation: InvocationId,
    },
    /// A bounded external import or operator-supplied source reference.
    External {
        /// Opaque durable source identity; never source bytes or credentials.
        source: CausalId,
    },
}

impl CausalReference {
    fn references_artifact(&self, artifact: &ArtifactReference) -> bool {
        matches!(self, Self::Artifact { reference } if reference == artifact)
    }
}

/// Bounded immutable producer and causal-input facts for an artifact.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ArtifactProvenance {
    producer: CausalReference,
    causes: Vec<CausalReference>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ArtifactProvenanceWire {
    producer: CausalReference,
    causes: Vec<CausalReference>,
}

milkdrift_contracts::deserialize_via!(
    ArtifactProvenance,
    ArtifactProvenanceWire,
    |wire| Self::new(wire.producer, wire.causes)
);

impl ArtifactProvenance {
    /// Constructs bounded provenance and rejects duplicate causal references.
    pub fn new(
        producer: CausalReference,
        causes: Vec<CausalReference>,
    ) -> Result<Self, WorkspaceError> {
        if causes.len() > MAX_CAUSAL_REFERENCES {
            return Err(WorkspaceError::InvalidArtifact(format!(
                "provenance may contain at most {MAX_CAUSAL_REFERENCES} causal references"
            )));
        }
        let unique: BTreeSet<_> = causes.iter().collect();
        if unique.len() != causes.len() {
            return Err(WorkspaceError::InvalidArtifact(
                "provenance cannot contain duplicate causal references".to_owned(),
            ));
        }
        Ok(Self { producer, causes })
    }

    /// Returns the exact producer fact.
    #[must_use]
    pub const fn producer(&self) -> &CausalReference {
        &self.producer
    }

    /// Returns the ordered causal input facts.
    #[must_use]
    pub fn causes(&self) -> &[CausalReference] {
        &self.causes
    }
}

/// Immutable metadata required to publish and later authorize an artifact.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ArtifactMetadata {
    reference: ArtifactReference,
    sensitivity: ArtifactSensitivity,
    retention: ArtifactRetention,
    provenance: ArtifactProvenance,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ArtifactMetadataWire {
    reference: ArtifactReference,
    #[serde(default)]
    sensitivity: ArtifactSensitivity,
    retention: ArtifactRetention,
    provenance: ArtifactProvenance,
}

milkdrift_contracts::deserialize_via!(ArtifactMetadata, ArtifactMetadataWire, |wire| Self::new(
    wire.reference,
    wire.sensitivity,
    wire.retention,
    wire.provenance,
));

impl ArtifactMetadata {
    /// Constructs complete metadata and rejects direct self-referential provenance.
    pub fn new(
        reference: ArtifactReference,
        sensitivity: ArtifactSensitivity,
        retention: ArtifactRetention,
        provenance: ArtifactProvenance,
    ) -> Result<Self, WorkspaceError> {
        if provenance.producer().references_artifact(&reference)
            || provenance
                .causes()
                .iter()
                .any(|cause| cause.references_artifact(&reference))
        {
            return Err(WorkspaceError::InvalidArtifact(
                "artifact provenance cannot directly reference the artifact itself".to_owned(),
            ));
        }
        Ok(Self {
            reference,
            sensitivity,
            retention,
            provenance,
        })
    }

    /// Returns the exact content reference.
    #[must_use]
    pub const fn reference(&self) -> &ArtifactReference {
        &self.reference
    }

    /// Returns the export-sensitivity classification.
    #[must_use]
    pub const fn sensitivity(&self) -> ArtifactSensitivity {
        self.sensitivity
    }

    /// Returns the minimum retention policy.
    #[must_use]
    pub const fn retention(&self) -> &ArtifactRetention {
        &self.retention
    }

    /// Returns the bounded immutable provenance.
    #[must_use]
    pub const fn provenance(&self) -> &ArtifactProvenance {
        &self.provenance
    }

    /// Verifies both the exact size and digest of complete content bytes.
    #[must_use]
    pub fn verifies(&self, bytes: &[u8]) -> bool {
        self.reference.verifies(bytes)
    }
}
