use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

use milkdrift_authority::SecretRef;
use milkdrift_capability::{
    AdmissionConstraints, BoundedJson, CancellationBehavior, CapabilityCategory,
    CapabilityDescriptor, CapabilityId, DescriptorBuilder, ExecutionTrustClass, ExtensionKey,
    FeatureContract, FeatureId, IdempotencyBehavior, Locality, OperationContract, OperationId,
    ProviderProfileRef, SchemaContract, SchemaId, SideEffectClass, StreamingMode, TrustZone,
};
use milkdrift_capability_host::MaterializationLimits;
use milkdrift_contracts::{JsonLimits, canonical_json_bytes, parse_json_without_duplicates};
use milkdrift_workspace::MediaType;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Value, json};
use thiserror::Error;

/// Legacy path-only schema, retained only so refusal can be explicit.
pub const PROCESS_PROFILE_SCHEMA_VERSION_V1: u32 = 1;
/// Exact current byte-pinned trusted-host process-profile document schema.
pub const PROCESS_PROFILE_SCHEMA_VERSION_V2: u32 = 2;
/// Maximum encoded profile document size.
pub const MAX_PROCESS_PROFILE_BYTES: usize = 1_048_576;
/// Maximum executable size accepted for bounded streaming identity verification.
pub const MAX_EXECUTABLE_BYTES: u64 = 1_073_741_824;
const MAX_ARGUMENTS: usize = 256;
const MAX_SUBSTITUTIONS: usize = 256;
const MAX_INPUT_FILES: usize = 120;
const MAX_OUTPUTS: usize = 120;
const MAX_ENVIRONMENT_VARIABLES: usize = 256;
const MAX_EXTENSIONS: usize = 64;
const MAX_EXTENSION_BYTES: usize = 65_536;
const MAX_REPORTS: u64 = 1_024;

/// Invalid or unsupported process profile.
#[derive(Debug, Error)]
pub enum ProcessProfileError {
    /// JSON syntax, duplicate keys, or typed decoding failed.
    #[error("invalid process profile JSON: {0}")]
    Json(String),
    /// The exact document version is unsupported.
    #[error("unsupported process profile schema version {found}; supported version is 2")]
    UnsupportedVersion {
        /// Observed version.
        found: u32,
    },
    /// A private semantic invariant was violated.
    #[error("invalid process profile: {0}")]
    Invalid(String),
    /// Descriptor generation failed.
    #[error("invalid process capability descriptor: {0}")]
    Descriptor(String),
}

/// Versioned configuration envelope. Secret values never appear in this document.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProcessProfileDocument {
    schema_version: u32,
    profile: ProcessProfile,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ProcessProfileDocumentWire {
    schema_version: u32,
    profile: ProcessProfile,
}

impl<'de> Deserialize<'de> for ProcessProfileDocument {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = ProcessProfileDocumentWire::deserialize(deserializer)?;
        if wire.schema_version != PROCESS_PROFILE_SCHEMA_VERSION_V2 {
            return Err(serde::de::Error::custom(format!(
                "unsupported process profile schema version {}",
                wire.schema_version
            )));
        }
        wire.profile.validate().map_err(serde::de::Error::custom)?;
        Ok(Self {
            schema_version: wire.schema_version,
            profile: wire.profile,
        })
    }
}

impl ProcessProfileDocument {
    /// Parses a duplicate-safe, size-bounded exact-v2 profile document.
    pub fn from_json(bytes: &[u8]) -> Result<Self, ProcessProfileError> {
        if bytes.len() > MAX_PROCESS_PROFILE_BYTES {
            return Err(ProcessProfileError::Invalid(
                "profile document exceeds one MiB".to_owned(),
            ));
        }
        let value = parse_json_without_duplicates(bytes)
            .map_err(|error| ProcessProfileError::Json(error.to_string()))?;
        let version = value
            .get("schema_version")
            .and_then(Value::as_u64)
            .and_then(|value| u32::try_from(value).ok())
            .ok_or_else(|| {
                ProcessProfileError::Json("missing numeric schema_version".to_owned())
            })?;
        if version != PROCESS_PROFILE_SCHEMA_VERSION_V2 {
            return Err(ProcessProfileError::UnsupportedVersion { found: version });
        }
        serde_json::from_value(value).map_err(|error| ProcessProfileError::Json(error.to_string()))
    }

    /// Recursively key-sorted compact JSON for compatibility fixtures and digesting.
    pub fn to_canonical_json(&self) -> Result<Vec<u8>, ProcessProfileError> {
        canonical_json_bytes(
            self,
            JsonLimits {
                maximum_depth: 48,
                maximum_string_bytes: 32_768,
                maximum_key_bytes: 192,
                maximum_container_items: 4_096,
            },
        )
        .map_err(|error| ProcessProfileError::Json(format!("{error:?}")))
    }

    /// Exact schema version.
    #[must_use]
    pub const fn schema_version(&self) -> u32 {
        self.schema_version
    }

    /// Validated process profile.
    #[must_use]
    pub const fn profile(&self) -> &ProcessProfile {
        &self.profile
    }

    /// Consumes the envelope and returns its validated profile.
    #[must_use]
    pub fn into_profile(self) -> ProcessProfile {
        self.profile
    }
}

/// Fixed source used by one explicitly named argument placeholder.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case", tag = "type", deny_unknown_fields)]
pub enum SubstitutionSource {
    /// Bounded inline invocation input rendered as one string argument.
    InputText {
        /// Exact invocation input name.
        input: String,
    },
    /// Absolute path of one selected materialized input.
    InputPath {
        /// Exact selected materialized input name.
        input: String,
    },
    /// Non-secret literal fixed by the profile revision.
    ConfigValue {
        /// Bounded non-secret literal value.
        value: String,
    },
    /// Canonical isolated execution root.
    ExecutionRoot,
    /// Stable invocation identity.
    InvocationId,
    /// Stable runtime idempotency key; invocation must supply it.
    IdempotencyKey,
}

/// How the child's working directory is selected beneath its isolated root.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case", tag = "type", deny_unknown_fields)]
pub enum WorkingDirectoryMode {
    /// Use the isolated root itself.
    IsolatedRoot,
    /// Use a declared relative directory beneath the isolated root.
    IsolatedSubdirectory {
        /// Directory created beneath the isolated root before entry.
        relative_path: PathBuf,
    },
}

/// Filesystem access fact used for executable and workspace-root validation.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FilesystemAccessMode {
    /// Executables may be entered from this root; no isolation claim is implied.
    Execute,
    /// Adapter-owned input reads are permitted.
    ReadOnly,
    /// Isolated workspaces may be created and mutated beneath this root.
    ReadWrite,
}

/// One canonicalization boundary and its intended access mode.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FilesystemRoot {
    pub(crate) path: PathBuf,
    pub(crate) access: FilesystemAccessMode,
}

/// Selected invocation input copied into the isolated workspace.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct InputFileRule {
    pub(crate) input: String,
    pub(crate) relative_path: PathBuf,
}

/// Explicit environment mediation policy.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EnvironmentPolicy {
    pub(crate) allowed_non_secret: BTreeSet<String>,
    pub(crate) secrets: BTreeMap<String, SecretRef>,
    pub(crate) max_value_bytes: usize,
}

/// Operator-declared immutable identity expected at the executable path.
///
/// The optional package revision adds operator provenance; it never substitutes
/// for exact byte hashing for this local-executable source.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutableIdentityDeclaration {
    pub(crate) content_digest: String,
    pub(crate) size_bytes: u64,
    pub(crate) package_revision: Option<String>,
    pub(crate) documentation_reference: Option<String>,
}

impl ExecutableIdentityDeclaration {
    /// Expected domain-labelled BLAKE3 digest of the executable bytes.
    #[must_use]
    pub fn content_digest(&self) -> &str {
        &self.content_digest
    }

    /// Expected exact executable byte size.
    #[must_use]
    pub const fn size_bytes(&self) -> u64 {
        self.size_bytes
    }

    /// Optional operator-declared immutable package or deployment revision.
    #[must_use]
    pub fn package_revision(&self) -> Option<&str> {
        self.package_revision.as_deref()
    }

    /// Optional bounded documentation reference.
    #[must_use]
    pub fn documentation_reference(&self) -> Option<&str> {
        self.documentation_reference.as_deref()
    }
}

/// Host-verified executable evidence used to construct one immutable descriptor.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct VerifiedExecutableIdentity {
    pub(crate) identity_digest: String,
    pub(crate) configured_path_digest: String,
    pub(crate) canonical_path_digest: String,
    pub(crate) content_digest: String,
    pub(crate) size_bytes: u64,
    pub(crate) package_revision: Option<String>,
    pub(crate) documentation_reference: Option<String>,
    pub(crate) platform: ExecutablePlatformEvidence,
}

/// Platform observations that affect entry compatibility but are never sole identity.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ExecutablePlatformEvidence {
    pub(crate) regular_file: bool,
    pub(crate) unix_mode: Option<u32>,
}

/// Bounded stdin source.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case", tag = "type", deny_unknown_fields)]
pub enum StdinMode {
    /// Child receives a closed/null stdin.
    Disabled,
    /// Exact selected materialized input bytes are written then stdin is closed.
    Input {
        /// Exact selected materialized input name.
        input: String,
        /// Maximum bytes written to stdin.
        max_bytes: u64,
    },
}

/// Action taken after a stdout/stderr capture bound is exceeded.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OverflowAction {
    /// Drain and discard excess bytes while allowing the process to continue.
    ContinueTruncated,
    /// Request termination and report a typed overflow failure.
    Terminate,
}

/// Independent bounded stdout or stderr policy.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CapturePolicy {
    pub(crate) max_capture_bytes: u64,
    pub(crate) stream_progress: bool,
    pub(crate) max_progress_events: u16,
    pub(crate) overflow_action: OverflowAction,
    pub(crate) artifact_name: Option<String>,
}

/// One declared regular-file output manifest rule.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OutputRule {
    pub(crate) name: String,
    pub(crate) relative_path: PathBuf,
    pub(crate) media_type: String,
    pub(crate) required: bool,
}

/// Recovery policy advertised by the configured executable contract.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RestartPolicy {
    /// A lost process remains uncertain and is never inferred complete.
    RetainUncertain,
    /// Retry is permitted only with the exact stable idempotency key.
    RetryWithStableKey,
}

/// Honest platform process-ownership facts frozen into the profile revision.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PlatformSupport {
    pub(crate) owned_process_group: bool,
    pub(crate) descendant_escape_prevention: bool,
    pub(crate) terminal_group_observation: bool,
}

impl PlatformSupport {
    /// Facts implemented by this build target.
    #[must_use]
    pub const fn current() -> Self {
        #[cfg(unix)]
        {
            Self {
                owned_process_group: true,
                descendant_escape_prevention: false,
                terminal_group_observation: true,
            }
        }
        #[cfg(not(unix))]
        {
            Self {
                owned_process_group: false,
                descendant_escape_prevention: false,
                terminal_group_observation: false,
            }
        }
    }

    /// Whether this build owns a dedicated group for the spawned tree.
    #[must_use]
    pub const fn owned_process_group(self) -> bool {
        self.owned_process_group
    }

    /// Whether descendants are prevented from escaping ownership.
    #[must_use]
    pub const fn descendant_escape_prevention(self) -> bool {
        self.descendant_escape_prevention
    }

    /// Whether group disappearance can be observed after termination.
    #[must_use]
    pub const fn terminal_group_observation(self) -> bool {
        self.terminal_group_observation
    }
}

/// Count, byte, path, and timing ceilings.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProcessLimits {
    pub(crate) max_argv_entries: u16,
    pub(crate) max_argv_bytes: u64,
    pub(crate) max_children_observed: u32,
    pub(crate) max_files: u32,
    pub(crate) max_file_bytes: u64,
    pub(crate) max_total_materialized_bytes: u64,
    pub(crate) max_path_bytes: usize,
    pub(crate) max_directory_depth: usize,
    pub(crate) artifact_chunk_bytes: u32,
    pub(crate) max_output_files: u16,
    pub(crate) max_total_output_bytes: u64,
    pub(crate) wall_timeout_ms: u64,
    pub(crate) graceful_termination_ms: u64,
    pub(crate) forced_termination_ms: u64,
    pub(crate) heartbeat_interval_ms: u64,
}

impl ProcessLimits {
    pub(crate) fn materialization(&self) -> MaterializationLimits {
        MaterializationLimits {
            max_files: self.max_files,
            max_file_bytes: self.max_file_bytes,
            max_total_bytes: self.max_total_materialized_bytes,
            max_path_bytes: self.max_path_bytes,
            max_directory_depth: self.max_directory_depth,
            chunk_bytes: self.artifact_chunk_bytes,
        }
    }
}

/// Validated immutable local-process profile revision.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProcessProfile {
    pub(crate) profile_id: String,
    pub(crate) revision: u64,
    pub(crate) capability: CapabilityId,
    pub(crate) descriptor_revision: u64,
    pub(crate) provider_profile: Option<ProviderProfileRef>,
    pub(crate) operation: OperationId,
    pub(crate) side_effect: SideEffectClass,
    pub(crate) idempotency: IdempotencyBehavior,
    pub(crate) cancellation: CancellationBehavior,
    pub(crate) trust_class: ExecutionTrustClass,
    pub(crate) executable: PathBuf,
    pub(crate) implementation: ExecutableIdentityDeclaration,
    pub(crate) arguments: Vec<String>,
    pub(crate) substitutions: BTreeMap<String, SubstitutionSource>,
    pub(crate) working_directory: WorkingDirectoryMode,
    pub(crate) filesystem_roots: Vec<FilesystemRoot>,
    pub(crate) inputs: Vec<InputFileRule>,
    pub(crate) environment: EnvironmentPolicy,
    pub(crate) stdin: StdinMode,
    pub(crate) stdout: CapturePolicy,
    pub(crate) stderr: CapturePolicy,
    pub(crate) outputs: Vec<OutputRule>,
    pub(crate) limits: ProcessLimits,
    pub(crate) restart: RestartPolicy,
    pub(crate) platform: PlatformSupport,
    pub(crate) max_concurrent: u32,
    pub(crate) extensions: BTreeMap<String, Value>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ProcessProfileWire {
    profile_id: String,
    revision: u64,
    capability: CapabilityId,
    descriptor_revision: u64,
    provider_profile: Option<ProviderProfileRef>,
    operation: OperationId,
    side_effect: SideEffectClass,
    idempotency: IdempotencyBehavior,
    cancellation: CancellationBehavior,
    trust_class: ExecutionTrustClass,
    executable: PathBuf,
    implementation: ExecutableIdentityDeclaration,
    arguments: Vec<String>,
    substitutions: BTreeMap<String, SubstitutionSource>,
    working_directory: WorkingDirectoryMode,
    filesystem_roots: Vec<FilesystemRoot>,
    inputs: Vec<InputFileRule>,
    environment: EnvironmentPolicy,
    stdin: StdinMode,
    stdout: CapturePolicy,
    stderr: CapturePolicy,
    outputs: Vec<OutputRule>,
    limits: ProcessLimits,
    restart: RestartPolicy,
    platform: PlatformSupport,
    max_concurrent: u32,
    extensions: BTreeMap<String, Value>,
}

impl<'de> Deserialize<'de> for ProcessProfile {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = ProcessProfileWire::deserialize(deserializer)?;
        let profile = Self {
            profile_id: wire.profile_id,
            revision: wire.revision,
            capability: wire.capability,
            descriptor_revision: wire.descriptor_revision,
            provider_profile: wire.provider_profile,
            operation: wire.operation,
            side_effect: wire.side_effect,
            idempotency: wire.idempotency,
            cancellation: wire.cancellation,
            trust_class: wire.trust_class,
            executable: wire.executable,
            implementation: wire.implementation,
            arguments: wire.arguments,
            substitutions: wire.substitutions,
            working_directory: wire.working_directory,
            filesystem_roots: wire.filesystem_roots,
            inputs: wire.inputs,
            environment: wire.environment,
            stdin: wire.stdin,
            stdout: wire.stdout,
            stderr: wire.stderr,
            outputs: wire.outputs,
            limits: wire.limits,
            restart: wire.restart,
            platform: wire.platform,
            max_concurrent: wire.max_concurrent,
            extensions: wire.extensions,
        };
        profile.validate().map_err(serde::de::Error::custom)?;
        Ok(profile)
    }
}

impl ProcessProfile {
    /// Stable configured profile identity.
    #[must_use]
    pub fn profile_id(&self) -> &str {
        &self.profile_id
    }

    /// Immutable profile revision.
    #[must_use]
    pub const fn revision(&self) -> u64 {
        self.revision
    }

    /// Exact capability identity.
    #[must_use]
    pub const fn capability(&self) -> &CapabilityId {
        &self.capability
    }

    /// Exact descriptor revision generated by this profile.
    #[must_use]
    pub const fn descriptor_revision(&self) -> u64 {
        self.descriptor_revision
    }

    /// Namespaced process operation.
    #[must_use]
    pub const fn operation(&self) -> &OperationId {
        &self.operation
    }

    /// Configured executable identity before host canonicalization.
    #[must_use]
    pub fn executable(&self) -> &Path {
        &self.executable
    }

    /// Exact operator-declared executable byte identity.
    #[must_use]
    pub const fn implementation(&self) -> &ExecutableIdentityDeclaration {
        &self.implementation
    }

    /// Exact execution-isolation/trust class.
    #[must_use]
    pub const fn trust_class(&self) -> ExecutionTrustClass {
        self.trust_class
    }

    /// Advertised restart behavior.
    #[must_use]
    pub const fn restart_policy(&self) -> RestartPolicy {
        self.restart
    }

    /// Honest platform support facts.
    #[must_use]
    pub const fn platform_support(&self) -> PlatformSupport {
        self.platform
    }

    /// Generates an immutable, capability-host-compatible descriptor.
    pub(crate) fn descriptor(
        &self,
        implementation: &VerifiedExecutableIdentity,
    ) -> Result<CapabilityDescriptor, ProcessProfileError> {
        let profile_digest = self.profile_digest()?;
        let execution_policy_digest = self.execution_policy_digest()?;
        let schema_value = BoundedJson::new(json!({
            "type": "object",
            "additionalProperties": true,
            "x-milkdrift-process-profile-schema": PROCESS_PROFILE_SCHEMA_VERSION_V2
        }))
        .map_err(|error| ProcessProfileError::Descriptor(error.to_string()))?;
        let input = SchemaContract::new(
            SchemaId::new("milkdrift.process.input")
                .map_err(|error| ProcessProfileError::Descriptor(error.to_string()))?,
            1,
            schema_value.clone(),
        )
        .map_err(|error| ProcessProfileError::Descriptor(error.to_string()))?;
        let output = SchemaContract::new(
            SchemaId::new("milkdrift.process.output")
                .map_err(|error| ProcessProfileError::Descriptor(error.to_string()))?,
            1,
            schema_value,
        )
        .map_err(|error| ProcessProfileError::Descriptor(error.to_string()))?;
        let mut streaming = BTreeSet::new();
        streaming.insert(
            if self.stdout.stream_progress || self.stderr.stream_progress {
                StreamingMode::Progress
            } else {
                StreamingMode::None
            },
        );
        let feature_ids = [
            "milkdrift.process.argv",
            "milkdrift.process.materialized-inputs",
            "milkdrift.process.declared-outputs",
        ];
        let mut features = BTreeMap::new();
        for identity in feature_ids {
            let identity = FeatureId::new(identity)
                .map_err(|error| ProcessProfileError::Descriptor(error.to_string()))?;
            features.insert(identity.clone(), FeatureContract::new(identity, None));
        }
        if self.platform.owned_process_group {
            let identity = FeatureId::new("milkdrift.process.owned-group")
                .map_err(|error| ProcessProfileError::Descriptor(error.to_string()))?;
            features.insert(identity.clone(), FeatureContract::new(identity, None));
        }
        let operation = OperationContract::new(
            input,
            output,
            streaming,
            self.cancellation,
            self.idempotency,
            self.side_effect,
            features,
        )
        .map_err(|error| ProcessProfileError::Descriptor(error.to_string()))?;
        let extension_key = ExtensionKey::new("org.milkdrift/process-profile")
            .map_err(|error| ProcessProfileError::Descriptor(error.to_string()))?;
        let extension_value = BoundedJson::new(json!({
            "schema_version": PROCESS_PROFILE_SCHEMA_VERSION_V2,
            "profile_id": self.profile_id,
            "profile_revision": self.revision,
            "profile_digest": profile_digest,
            "execution_policy_digest": execution_policy_digest,
            "execution_trust": self.trust_class,
            "implementation": implementation,
            "owned_process_group": self.platform.owned_process_group,
            "descendant_escape_prevention": self.platform.descendant_escape_prevention,
            "terminal_group_observation": self.platform.terminal_group_observation,
            "resource_limits_are_observational": true
        }))
        .map_err(|error| ProcessProfileError::Descriptor(error.to_string()))?;
        DescriptorBuilder::new(
            self.capability.clone(),
            self.descriptor_revision,
            CapabilityCategory::Process,
            AdmissionConstraints::new(self.max_concurrent, 0)
                .map_err(|error| ProcessProfileError::Descriptor(error.to_string()))?,
            Locality::Local,
        )
        .provider_profile(self.provider_profile.clone())
        .operations(BTreeMap::from([(self.operation.clone(), operation)]))
        .execution_trust(self.trust_class)
        .trust_zones(BTreeSet::from([TrustZone::new("local-process").map_err(
            |error| ProcessProfileError::Descriptor(error.to_string()),
        )?]))
        .labels(BTreeSet::from(["trusted host process".to_owned()]))
        .extensions(BTreeMap::from([(extension_key, extension_value)]))
        .build()
        .map_err(|error| ProcessProfileError::Descriptor(error.to_string()))
    }

    fn validate(&self) -> Result<(), ProcessProfileError> {
        validate_safe_name("profile_id", &self.profile_id)?;
        if self.revision == 0
            || self.descriptor_revision == 0
            || self.descriptor_revision != self.revision
            || self.max_concurrent == 0
        {
            return Err(ProcessProfileError::Invalid(
                "profile and descriptor revisions must be equal/nonzero and admission must be nonzero"
                    .to_owned(),
            ));
        }
        if self.trust_class != ExecutionTrustClass::TrustedHostProcess {
            return Err(ProcessProfileError::Invalid(
                "the local-process adapter requires trust_class trusted_host_process".to_owned(),
            ));
        }
        validate_implementation(&self.implementation)?;
        if !self.executable.is_absolute() || path_text(&self.executable)?.contains('\0') {
            return Err(ProcessProfileError::Invalid(
                "executable must be an absolute NUL-free path".to_owned(),
            ));
        }
        if self.arguments.is_empty()
            || self.arguments.len() > MAX_ARGUMENTS
            || self
                .arguments
                .iter()
                .any(|argument| argument.len() > 32_768 || argument.contains('\0'))
            || self.substitutions.len() > MAX_SUBSTITUTIONS
        {
            return Err(ProcessProfileError::Invalid(
                "argument vector/template bounds are invalid".to_owned(),
            ));
        }
        for (name, source) in &self.substitutions {
            validate_safe_name("substitution", name)?;
            match source {
                SubstitutionSource::InputText { input }
                | SubstitutionSource::InputPath { input } => validate_safe_name("input", input)?,
                SubstitutionSource::ConfigValue { value }
                    if value.len() > 32_768 || value.contains('\0') =>
                {
                    return Err(ProcessProfileError::Invalid(
                        "config substitution violates its bound".to_owned(),
                    ));
                }
                SubstitutionSource::ConfigValue { .. }
                | SubstitutionSource::ExecutionRoot
                | SubstitutionSource::InvocationId
                | SubstitutionSource::IdempotencyKey => {}
            }
        }
        for argument in &self.arguments {
            for placeholder in placeholders(argument)? {
                if !self.substitutions.contains_key(placeholder) {
                    return Err(ProcessProfileError::Invalid(format!(
                        "argument references unknown placeholder '{placeholder}'"
                    )));
                }
            }
        }
        if self.filesystem_roots.is_empty()
            || !self
                .filesystem_roots
                .iter()
                .any(|root| root.access == FilesystemAccessMode::Execute)
            || !self
                .filesystem_roots
                .iter()
                .any(|root| root.access == FilesystemAccessMode::ReadWrite)
        {
            return Err(ProcessProfileError::Invalid(
                "filesystem roots require executable and read-write entries".to_owned(),
            ));
        }
        for root in &self.filesystem_roots {
            if !root.path.is_absolute() || path_text(&root.path)?.contains('\0') {
                return Err(ProcessProfileError::Invalid(
                    "filesystem roots must be absolute NUL-free paths".to_owned(),
                ));
            }
        }
        if self.inputs.len() > MAX_INPUT_FILES || self.outputs.len() > MAX_OUTPUTS {
            return Err(ProcessProfileError::Invalid(
                "input/output rule count exceeds the profile bound".to_owned(),
            ));
        }
        let mut input_names = BTreeSet::new();
        let mut input_paths = BTreeSet::new();
        for input in &self.inputs {
            validate_safe_name("input", &input.input)?;
            validate_relative_path(&input.relative_path, &self.limits)?;
            if !input_names.insert(&input.input) || !input_paths.insert(&input.relative_path) {
                return Err(ProcessProfileError::Invalid(
                    "input materialization names and paths must be unique".to_owned(),
                ));
            }
        }
        for source in self.substitutions.values() {
            if let SubstitutionSource::InputPath { input } = source
                && !input_names.contains(input)
            {
                return Err(ProcessProfileError::Invalid(format!(
                    "input-path substitution '{input}' is not materialized"
                )));
            }
        }
        if let StdinMode::Input { input, max_bytes } = &self.stdin
            && (*max_bytes == 0
                || *max_bytes > self.limits.max_file_bytes
                || !input_names.contains(input))
        {
            return Err(ProcessProfileError::Invalid(
                "stdin input must be materialized with a valid byte bound".to_owned(),
            ));
        }
        validate_environment(&self.environment)?;
        validate_capture("stdout", &self.stdout, !self.environment.secrets.is_empty())?;
        validate_capture("stderr", &self.stderr, !self.environment.secrets.is_empty())?;
        let mut output_names = BTreeSet::new();
        let mut output_paths = BTreeSet::new();
        for output in &self.outputs {
            validate_safe_name("output", &output.name)?;
            validate_relative_path(&output.relative_path, &self.limits)?;
            MediaType::new(output.media_type.clone())
                .map_err(|error| ProcessProfileError::Invalid(error.to_string()))?;
            if !output_names.insert(&output.name) || !output_paths.insert(&output.relative_path) {
                return Err(ProcessProfileError::Invalid(
                    "output manifest names and paths must be unique".to_owned(),
                ));
            }
        }
        for capture in [&self.stdout, &self.stderr] {
            if let Some(name) = &capture.artifact_name {
                validate_safe_name("capture artifact", name)?;
                if !output_names.insert(name) {
                    return Err(ProcessProfileError::Invalid(
                        "capture and manifest output names must be unique".to_owned(),
                    ));
                }
            }
        }
        self.limits
            .materialization()
            .validate()
            .map_err(|error| ProcessProfileError::Invalid(error.to_string()))?;
        if self.limits.max_argv_entries == 0
            || usize::from(self.limits.max_argv_entries) > MAX_ARGUMENTS
            || self.limits.max_argv_bytes == 0
            || self.limits.max_children_observed == 0
            || self.limits.max_output_files == 0
            || usize::from(self.limits.max_output_files) < output_names.len()
            || self.limits.max_total_output_bytes == 0
            || self.limits.wall_timeout_ms == 0
            || self.limits.graceful_termination_ms == 0
            || self.limits.forced_termination_ms == 0
            || self.limits.heartbeat_interval_ms == 0
        {
            return Err(ProcessProfileError::Invalid(
                "process timing/count/output bounds are invalid".to_owned(),
            ));
        }
        let heartbeat_reports = self
            .limits
            .wall_timeout_ms
            .div_ceil(self.limits.heartbeat_interval_ms);
        let maximum_reports = heartbeat_reports
            .saturating_add(u64::from(self.stdout.max_progress_events))
            .saturating_add(u64::from(self.stderr.max_progress_events))
            .saturating_add(output_names.len() as u64)
            .saturating_add(2);
        if maximum_reports > MAX_REPORTS {
            return Err(ProcessProfileError::Invalid(
                "heartbeat/progress/output policy can exceed the durable reporter bound".to_owned(),
            ));
        }
        if self.cancellation != CancellationBehavior::BestEffort {
            return Err(ProcessProfileError::Invalid(
                "local process cancellation is honestly advertised as best-effort".to_owned(),
            ));
        }
        if self.side_effect == SideEffectClass::IdempotentWrite
            && (self.idempotency == IdempotencyBehavior::Unsupported
                || !self
                    .substitutions
                    .values()
                    .any(|source| matches!(source, SubstitutionSource::IdempotencyKey)))
        {
            return Err(ProcessProfileError::Invalid(
                "idempotent writes require an advertised and argv-scoped stable key".to_owned(),
            ));
        }
        if self.restart == RestartPolicy::RetryWithStableKey
            && (self.idempotency == IdempotencyBehavior::Unsupported
                || !self
                    .substitutions
                    .values()
                    .any(|source| matches!(source, SubstitutionSource::IdempotencyKey)))
        {
            return Err(ProcessProfileError::Invalid(
                "restart retry requires an executable contract using the stable key".to_owned(),
            ));
        }
        if self.platform != PlatformSupport::current() {
            return Err(ProcessProfileError::Invalid(
                "platform support facts do not match this build target".to_owned(),
            ));
        }
        if self.extensions.len() > MAX_EXTENSIONS
            || self.extensions.keys().any(|key| {
                key.len() > 192 || !key.contains('/') || !key.is_ascii() || key.contains('\0')
            })
            || serde_json::to_vec(&self.extensions)
                .map_err(|error| ProcessProfileError::Json(error.to_string()))?
                .len()
                > MAX_EXTENSION_BYTES
        {
            return Err(ProcessProfileError::Invalid(
                "profile extensions must be bounded and DNS-namespaced".to_owned(),
            ));
        }
        match &self.working_directory {
            WorkingDirectoryMode::IsolatedRoot => {}
            WorkingDirectoryMode::IsolatedSubdirectory { relative_path } => {
                validate_relative_path(relative_path, &self.limits)?;
            }
        }
        Ok(())
    }

    fn profile_digest(&self) -> Result<String, ProcessProfileError> {
        let bytes = canonical_json_bytes(
            self,
            JsonLimits {
                maximum_depth: 48,
                maximum_string_bytes: 32_768,
                maximum_key_bytes: 192,
                maximum_container_items: 4_096,
            },
        )
        .map_err(|error| ProcessProfileError::Json(format!("{error:?}")))?;
        let mut hasher = blake3::Hasher::new();
        hasher.update(b"milkdrift.process-profile.v2\0");
        hasher.update(&bytes);
        Ok(format!("b3_{}", hasher.finalize()))
    }

    fn execution_policy_digest(&self) -> Result<String, ProcessProfileError> {
        let policy = json!({
            "schema_version": PROCESS_PROFILE_SCHEMA_VERSION_V2,
            "operation": self.operation,
            "side_effect": self.side_effect,
            "idempotency": self.idempotency,
            "cancellation": self.cancellation,
            "trust_class": self.trust_class,
            "arguments": self.arguments,
            "substitutions": self.substitutions,
            "working_directory": self.working_directory,
            "filesystem_roots": self.filesystem_roots,
            "inputs": self.inputs,
            "environment": self.environment,
            "stdin": self.stdin,
            "stdout": self.stdout,
            "stderr": self.stderr,
            "outputs": self.outputs,
            "limits": self.limits,
            "restart": self.restart,
            "platform": self.platform,
            "max_concurrent": self.max_concurrent,
        });
        let bytes = canonical_json_bytes(
            &policy,
            JsonLimits {
                maximum_depth: 48,
                maximum_string_bytes: 32_768,
                maximum_key_bytes: 192,
                maximum_container_items: 4_096,
            },
        )
        .map_err(|error| ProcessProfileError::Json(format!("{error:?}")))?;
        let mut hasher = blake3::Hasher::new();
        hasher.update(b"milkdrift.process-execution-policy.v1\0");
        hasher.update(&bytes);
        Ok(format!("b3_{}", hasher.finalize()))
    }
}

fn validate_implementation(
    implementation: &ExecutableIdentityDeclaration,
) -> Result<(), ProcessProfileError> {
    if !valid_blake3_digest(&implementation.content_digest)
        || implementation.size_bytes == 0
        || implementation.size_bytes > MAX_EXECUTABLE_BYTES
    {
        return Err(ProcessProfileError::Invalid(
            "implementation requires a b3_ digest and bounded nonzero executable size".to_owned(),
        ));
    }
    if implementation
        .package_revision
        .as_ref()
        .is_some_and(|value| {
            value.is_empty()
                || value.len() > 256
                || !value.is_ascii()
                || value.bytes().any(|byte| byte.is_ascii_control())
        })
    {
        return Err(ProcessProfileError::Invalid(
            "package revision must contain 1..=256 printable ASCII bytes".to_owned(),
        ));
    }
    if implementation
        .documentation_reference
        .as_ref()
        .is_some_and(|value| {
            value.is_empty()
                || value.len() > 1_024
                || value.contains('\0')
                || value.chars().any(char::is_control)
        })
    {
        return Err(ProcessProfileError::Invalid(
            "documentation reference must contain 1..=1024 printable bytes".to_owned(),
        ));
    }
    Ok(())
}

fn valid_blake3_digest(value: &str) -> bool {
    value.strip_prefix("b3_").is_some_and(|hex| {
        hex.len() == 64
            && hex
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    })
}

fn validate_environment(policy: &EnvironmentPolicy) -> Result<(), ProcessProfileError> {
    if policy
        .allowed_non_secret
        .len()
        .saturating_add(policy.secrets.len())
        > MAX_ENVIRONMENT_VARIABLES
        || policy.max_value_bytes == 0
        || policy.max_value_bytes > 1_048_576
    {
        return Err(ProcessProfileError::Invalid(
            "environment policy count/value bounds are invalid".to_owned(),
        ));
    }
    for name in policy
        .allowed_non_secret
        .iter()
        .chain(policy.secrets.keys())
    {
        validate_environment_name(name)?;
    }
    if policy
        .allowed_non_secret
        .iter()
        .any(|name| policy.secrets.contains_key(name))
    {
        return Err(ProcessProfileError::Invalid(
            "an environment name cannot be both secret and non-secret".to_owned(),
        ));
    }
    Ok(())
}

fn validate_capture(
    name: &str,
    policy: &CapturePolicy,
    has_secrets: bool,
) -> Result<(), ProcessProfileError> {
    if policy.max_capture_bytes > 64 * 1024 * 1024
        || (policy.stream_progress && policy.max_progress_events == 0)
        || (!policy.stream_progress && policy.max_progress_events != 0)
        || (policy.artifact_name.is_some() && policy.max_capture_bytes == 0)
        || (has_secrets && policy.stream_progress)
    {
        return Err(ProcessProfileError::Invalid(format!(
            "{name} capture/streaming policy is invalid; secret-bearing profiles cannot stream process text"
        )));
    }
    Ok(())
}

fn validate_environment_name(value: &str) -> Result<(), ProcessProfileError> {
    if value.is_empty()
        || value.len() > 128
        || !value.is_ascii()
        || value.contains('=')
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
    {
        return Err(ProcessProfileError::Invalid(
            "environment names must contain 1..=128 ASCII alphanumeric/'_' bytes".to_owned(),
        ));
    }
    Ok(())
}

fn validate_safe_name(kind: &str, value: &str) -> Result<(), ProcessProfileError> {
    if value.is_empty()
        || value.len() > 128
        || !value.is_ascii()
        || !value.as_bytes()[0].is_ascii_alphanumeric()
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
    {
        return Err(ProcessProfileError::Invalid(format!(
            "{kind} must contain 1..=128 safe ASCII bytes"
        )));
    }
    Ok(())
}

fn validate_relative_path(path: &Path, limits: &ProcessLimits) -> Result<(), ProcessProfileError> {
    let text = path_text(path)?;
    if text.is_empty()
        || text.len() > limits.max_path_bytes
        || text.contains('\0')
        || path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, std::path::Component::Normal(_)))
        || path.components().count() > limits.max_directory_depth
    {
        return Err(ProcessProfileError::Invalid(
            "relative path contains an escape/invalid component or exceeds its bounds".to_owned(),
        ));
    }
    Ok(())
}

fn path_text(path: &Path) -> Result<&str, ProcessProfileError> {
    path.to_str()
        .ok_or_else(|| ProcessProfileError::Invalid("profile paths must be valid UTF-8".to_owned()))
}

pub(crate) fn placeholders(template: &str) -> Result<Vec<&str>, ProcessProfileError> {
    let mut values = Vec::new();
    let mut rest = template;
    while let Some(start) = rest.find("{{") {
        let before = &rest[..start];
        if before.contains("}}") {
            return Err(ProcessProfileError::Invalid(
                "argument template contains an unmatched closing delimiter".to_owned(),
            ));
        }
        let after = &rest[start + 2..];
        let end = after.find("}}").ok_or_else(|| {
            ProcessProfileError::Invalid(
                "argument template contains an unmatched opening delimiter".to_owned(),
            )
        })?;
        let name = &after[..end];
        validate_safe_name("placeholder", name)?;
        values.push(name);
        rest = &after[end + 2..];
    }
    if rest.contains("}}") {
        return Err(ProcessProfileError::Invalid(
            "argument template contains an unmatched closing delimiter".to_owned(),
        ));
    }
    Ok(values)
}
