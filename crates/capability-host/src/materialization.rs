//! Host-owned materialization and artifact-publication boundary for concrete adapters.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Component, Path, PathBuf},
    sync::Arc,
};

use milkdrift_capability::{
    ArtifactReference as CapabilityArtifactReference, InputReference, InvocationRequest,
    InvocationValueReference,
};
use milkdrift_persistence::{
    ArtifactPublicationId, ArtifactReadAuthority, ArtifactReadRequest, BeginArtifactOutcome,
    BeginArtifactPublication, MAX_ARTIFACT_CHUNK_BYTES,
};
use milkdrift_runtime::RuntimeStore;
use milkdrift_workspace::{
    ArtifactId, ArtifactMetadata, ArtifactProvenance, ArtifactReference, ArtifactRetention,
    ArtifactSensitivity, CausalId, CausalReference, ContentDigest, MediaType, WorkspaceBudget,
    WorkspaceValue, WorkspaceValueReference,
};
use tempfile::TempDir;
use thiserror::Error;

use crate::AdapterExecutionContext;

/// Stable version of the host materialization contract.
pub const MATERIALIZATION_SCHEMA_VERSION_V1: u32 = 1;

/// Typed failure at the bounded invocation-data boundary.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum InvocationDataError {
    /// Input/profile facts were invalid before external process entry.
    #[error("materialization rejected: {0}")]
    Rejected(String),
    /// Durable input content was missing, malformed, or contradicted its reference.
    #[error("materialization integrity failure: {0}")]
    Integrity(String),
    /// A bounded filesystem operation failed.
    #[error("materialization filesystem failure: {0}")]
    Filesystem(String),
    /// Artifact publication failed; incomplete streams were aborted when possible.
    #[error("artifact publication failure: {0}")]
    Publication(String),
}

/// One explicitly selected invocation input and its isolated relative destination.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InputMaterialization {
    input_name: String,
    relative_path: PathBuf,
}

impl InputMaterialization {
    /// Validates one named input destination.
    pub fn new(
        input_name: impl Into<String>,
        relative_path: impl Into<PathBuf>,
    ) -> Result<Self, InvocationDataError> {
        let input_name = input_name.into();
        let relative_path = relative_path.into();
        if input_name.is_empty() || input_name.len() > 128 {
            return Err(InvocationDataError::Rejected(
                "input materialization name must contain 1..=128 bytes".to_owned(),
            ));
        }
        validate_relative_path(&relative_path, 4_096, 64)?;
        Ok(Self {
            input_name,
            relative_path,
        })
    }

    /// Invocation input name.
    #[must_use]
    pub fn input_name(&self) -> &str {
        &self.input_name
    }

    /// Destination beneath the isolated execution root.
    #[must_use]
    pub fn relative_path(&self) -> &Path {
        &self.relative_path
    }
}

/// Inclusive defensive bounds applied while materializing and importing files.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MaterializationLimits {
    /// Maximum selected input/output file count.
    pub max_files: u32,
    /// Maximum bytes in one selected input/output file.
    pub max_file_bytes: u64,
    /// Maximum aggregate selected input/output bytes.
    pub max_total_bytes: u64,
    /// Maximum platform path bytes/UTF-8 bytes for configured relative paths.
    pub max_path_bytes: usize,
    /// Maximum relative directory depth.
    pub max_directory_depth: usize,
    /// Bounded artifact I/O chunk size.
    pub chunk_bytes: u32,
}

impl MaterializationLimits {
    /// Validates nonzero and internally consistent materialization bounds.
    pub fn validate(self) -> Result<Self, InvocationDataError> {
        if self.max_files == 0
            || self.max_file_bytes == 0
            || self.max_total_bytes == 0
            || self.max_file_bytes > self.max_total_bytes
            || self.max_path_bytes == 0
            || self.max_path_bytes > 32_768
            || self.max_directory_depth == 0
            || self.max_directory_depth > 256
            || self.chunk_bytes == 0
            || usize::try_from(self.chunk_bytes)
                .map_or(true, |value| value > MAX_ARTIFACT_CHUNK_BYTES)
        {
            return Err(InvocationDataError::Rejected(
                "invalid materialization limits".to_owned(),
            ));
        }
        Ok(self)
    }
}

/// Opaque execution workspace leased by the data-access implementation.
///
/// Dropping the value releases implementation-owned temporary state.
pub trait MaterializedExecution: Send {
    /// Canonical isolated execution root.
    fn root(&self) -> &Path;
    /// Exact path of a selected materialized input.
    fn input_path(&self, input_name: &str) -> Option<&Path>;
}

/// Narrow host-owned input/materialization/output port used by process adapters.
pub trait InvocationDataAccess: Send + Sync {
    /// Reads and verifies one exact durable artifact without exposing store layout.
    fn read_artifact_bytes(
        &self,
        _context: &AdapterExecutionContext,
        reference: &CapabilityArtifactReference,
        _limits: MaterializationLimits,
    ) -> Result<Vec<u8>, InvocationDataError> {
        let _ = reference;
        Err(InvocationDataError::Rejected(
            "direct artifact reading is unsupported by this data-access implementation".to_owned(),
        ))
    }

    /// Reads exact durable inputs and creates one isolated execution workspace.
    fn materialize(
        &self,
        context: &AdapterExecutionContext,
        request: &InvocationRequest,
        inputs: &[InputMaterialization],
        limits: MaterializationLimits,
    ) -> Result<Box<dyn MaterializedExecution>, InvocationDataError>;

    /// Imports and publishes one declared regular output file.
    #[allow(clippy::too_many_arguments)]
    fn publish_file(
        &self,
        context: &AdapterExecutionContext,
        request: &InvocationRequest,
        workspace: &dyn MaterializedExecution,
        output_name: &str,
        relative_path: &Path,
        media_type: &str,
        limits: MaterializationLimits,
    ) -> Result<CapabilityArtifactReference, InvocationDataError>;

    /// Publishes one bounded adapter-owned byte capture without exposing store layout.
    fn publish_bytes(
        &self,
        context: &AdapterExecutionContext,
        request: &InvocationRequest,
        output_name: &str,
        media_type: &str,
        bytes: &[u8],
        limits: MaterializationLimits,
    ) -> Result<CapabilityArtifactReference, InvocationDataError>;
}

/// Production bridge over the injected runtime persistence ports.
pub struct StoreInvocationDataAccess {
    store: Arc<dyn RuntimeStore>,
    temporary_root: PathBuf,
    read_authority: ArtifactReadAuthority,
    workspace_budget: WorkspaceBudget,
}

impl StoreInvocationDataAccess {
    /// Creates a bridge rooted at one preconfigured canonical temporary directory.
    pub fn new(
        store: Arc<dyn RuntimeStore>,
        temporary_root: impl Into<PathBuf>,
        read_authority: ArtifactReadAuthority,
        workspace_budget: WorkspaceBudget,
    ) -> Result<Self, InvocationDataError> {
        let temporary_root = temporary_root.into();
        fs::create_dir_all(&temporary_root)
            .map_err(|error| fs_error("create temporary root", &error))?;
        let temporary_root = temporary_root
            .canonicalize()
            .map_err(|error| fs_error("canonicalize temporary root", &error))?;
        if !temporary_root.is_dir() {
            return Err(InvocationDataError::Rejected(
                "temporary execution root is not a directory".to_owned(),
            ));
        }
        Ok(Self {
            store,
            temporary_root,
            read_authority,
            workspace_budget,
        })
    }

    fn input_bytes(
        &self,
        input: &InputReference,
        limits: MaterializationLimits,
    ) -> Result<(Vec<u8>, CausalReference), InvocationDataError> {
        match input.value() {
            InvocationValueReference::Inline { value } => {
                let bytes = serde_json::to_vec(value.value())
                    .map_err(|error| InvocationDataError::Integrity(error.to_string()))?;
                let source = inline_cause(input.name(), &bytes)?;
                Ok((bytes, source))
            }
            InvocationValueReference::WorkspaceValue { identity, version } => {
                let reference: WorkspaceValueReference =
                    serde_json::from_str(identity).map_err(|_error| {
                        InvocationDataError::Integrity(format!(
                            "workspace input '{}' is not an exact reference",
                            input.name()
                        ))
                    })?;
                if version != &reference.version().get().to_string() {
                    return Err(InvocationDataError::Integrity(format!(
                        "workspace input '{}' has a contradictory version",
                        input.name()
                    )));
                }
                let entry = self
                    .store
                    .value(&reference)
                    .map_err(|error| InvocationDataError::Integrity(error.to_string()))?
                    .ok_or_else(|| {
                        InvocationDataError::Integrity(format!(
                            "workspace input '{}' is unavailable",
                            input.name()
                        ))
                    })?;
                let bytes = match entry.value() {
                    WorkspaceValue::Json(value) => serde_json::to_vec(value.value())
                        .map_err(|error| InvocationDataError::Integrity(error.to_string()))?,
                    WorkspaceValue::Artifact(reference) => {
                        self.read_artifact(reference.clone(), limits)?
                    }
                };
                Ok((bytes, CausalReference::WorkspaceValue { reference }))
            }
            InvocationValueReference::Artifact { reference } => {
                let durable = durable_artifact_reference(reference)?;
                let bytes = self.read_artifact(durable.clone(), limits)?;
                Ok((bytes, CausalReference::Artifact { reference: durable }))
            }
        }
    }

    fn read_artifact(
        &self,
        reference: ArtifactReference,
        limits: MaterializationLimits,
    ) -> Result<Vec<u8>, InvocationDataError> {
        if reference.size_bytes() > limits.max_file_bytes {
            return Err(InvocationDataError::Rejected(
                "artifact input exceeds the per-file materialization bound".to_owned(),
            ));
        }
        let capacity = usize::try_from(reference.size_bytes()).map_err(|_error| {
            InvocationDataError::Rejected("artifact size cannot fit this platform".to_owned())
        })?;
        let mut bytes = Vec::with_capacity(capacity);
        let mut offset = 0_u64;
        while offset < reference.size_bytes() {
            let remaining = reference.size_bytes().saturating_sub(offset);
            let maximum = u64::from(limits.chunk_bytes).min(remaining);
            let maximum = u32::try_from(maximum).map_err(|_error| {
                InvocationDataError::Rejected("artifact chunk bound overflow".to_owned())
            })?;
            let request = ArtifactReadRequest::new(
                reference.clone(),
                offset,
                maximum,
                self.read_authority.clone(),
            )
            .map_err(|error| InvocationDataError::Integrity(error.to_string()))?;
            let chunk = self
                .store
                .read_chunk(&request)
                .map_err(|error| InvocationDataError::Integrity(error.to_string()))?;
            if chunk.offset != offset || chunk.bytes.is_empty() {
                return Err(InvocationDataError::Integrity(
                    "artifact reader made no exact progress".to_owned(),
                ));
            }
            offset = offset
                .checked_add(u64::try_from(chunk.bytes.len()).map_err(|_error| {
                    InvocationDataError::Integrity("artifact chunk size overflow".to_owned())
                })?)
                .ok_or_else(|| {
                    InvocationDataError::Integrity("artifact offset overflow".to_owned())
                })?;
            bytes.extend_from_slice(&chunk.bytes);
            if chunk.end_of_artifact != (offset == reference.size_bytes()) {
                return Err(InvocationDataError::Integrity(
                    "artifact end marker contradicts exact size".to_owned(),
                ));
            }
        }
        if !reference.verifies(&bytes) {
            return Err(InvocationDataError::Integrity(
                "artifact bytes contradict their immutable reference".to_owned(),
            ));
        }
        Ok(bytes)
    }

    fn publish(
        &self,
        context: &AdapterExecutionContext,
        request: &InvocationRequest,
        output_name: &str,
        media_type: &str,
        bytes: &[u8],
        limits: MaterializationLimits,
    ) -> Result<CapabilityArtifactReference, InvocationDataError> {
        validate_output_name(output_name)?;
        if u64::try_from(bytes.len()).map_or(true, |size| size > limits.max_file_bytes) {
            return Err(InvocationDataError::Rejected(
                "declared output exceeds the per-file publication bound".to_owned(),
            ));
        }
        let digest = ContentDigest::for_bytes(bytes);
        let identity_hash = publication_hash(context, request, output_name, digest);
        let artifact = ArtifactId::new(format!("process:{}", identity_hash.to_hex()))
            .map_err(|error| InvocationDataError::Publication(error.to_string()))?;
        let reference = ArtifactReference::new(
            artifact,
            digest,
            MediaType::new(media_type.to_owned())
                .map_err(|error| InvocationDataError::Rejected(error.to_string()))?,
            u64::try_from(bytes.len()).map_err(|_error| {
                InvocationDataError::Rejected("output size cannot fit u64".to_owned())
            })?,
        );
        let provenance = ArtifactProvenance::new(
            CausalReference::Invocation {
                invocation: request.invocation().clone(),
            },
            publication_causes(context, request)?,
        )
        .map_err(|error| InvocationDataError::Publication(error.to_string()))?;
        let metadata = ArtifactMetadata::new(
            reference.clone(),
            ArtifactSensitivity::Restricted,
            ArtifactRetention::WhileReferenced,
            provenance,
        )
        .map_err(|error| InvocationDataError::Publication(error.to_string()))?;
        let publication =
            ArtifactPublicationId::new(format!("process-publication:{}", identity_hash.to_hex()))
                .map_err(|error| InvocationDataError::Publication(error.to_string()))?;
        let usage = self
            .store
            .workspace_usage(context.run())
            .map_err(|error| InvocationDataError::Publication(error.to_string()))?;
        let begin = BeginArtifactPublication::new(
            publication.clone(),
            context.run().clone(),
            metadata,
            self.workspace_budget.clone(),
            usage,
        )
        .map_err(|error| InvocationDataError::Publication(error.to_string()))?;
        let begin_outcome = self
            .store
            .begin_publication(&begin)
            .map_err(|error| InvocationDataError::Publication(error.to_string()))?;
        if let BeginArtifactOutcome::AlreadyCommitted(metadata) = begin_outcome {
            return capability_artifact_reference(metadata.reference());
        }
        let mut offset = begin_outcome.next_offset().unwrap_or(0);
        let start = usize::try_from(offset).map_err(|_error| {
            InvocationDataError::Publication("publication offset cannot fit usize".to_owned())
        })?;
        if start > bytes.len() {
            let _ = self.store.abort_publication(&publication);
            return Err(InvocationDataError::Publication(
                "resumed publication offset exceeds exact output size".to_owned(),
            ));
        }
        for chunk in bytes[start..].chunks(MAX_ARTIFACT_CHUNK_BYTES) {
            if let Err(error) = self.store.write_chunk(&publication, offset, chunk) {
                let _ = self.store.abort_publication(&publication);
                return Err(InvocationDataError::Publication(error.to_string()));
            }
            offset = offset
                .checked_add(u64::try_from(chunk.len()).map_err(|_error| {
                    InvocationDataError::Publication("publication chunk overflow".to_owned())
                })?)
                .ok_or_else(|| {
                    InvocationDataError::Publication("publication offset overflow".to_owned())
                })?;
        }
        let committed = match self.store.commit_publication(&publication) {
            Ok(committed) => committed,
            Err(error) => {
                let _ = self.store.abort_publication(&publication);
                return Err(InvocationDataError::Publication(error.to_string()));
            }
        };
        capability_artifact_reference(committed.metadata().reference())
    }
}

impl InvocationDataAccess for StoreInvocationDataAccess {
    fn read_artifact_bytes(
        &self,
        _context: &AdapterExecutionContext,
        reference: &CapabilityArtifactReference,
        limits: MaterializationLimits,
    ) -> Result<Vec<u8>, InvocationDataError> {
        self.read_artifact(durable_artifact_reference(reference)?, limits.validate()?)
    }

    fn materialize(
        &self,
        _context: &AdapterExecutionContext,
        request: &InvocationRequest,
        inputs: &[InputMaterialization],
        limits: MaterializationLimits,
    ) -> Result<Box<dyn MaterializedExecution>, InvocationDataError> {
        let limits = limits.validate()?;
        if u32::try_from(inputs.len()).map_or(true, |count| count > limits.max_files) {
            return Err(InvocationDataError::Rejected(
                "selected input count exceeds the materialization bound".to_owned(),
            ));
        }
        let mut names = BTreeSet::new();
        let mut paths = BTreeSet::new();
        for input in inputs {
            validate_relative_path(
                input.relative_path(),
                limits.max_path_bytes,
                limits.max_directory_depth,
            )?;
            if !names.insert(input.input_name()) || !paths.insert(input.relative_path()) {
                return Err(InvocationDataError::Rejected(
                    "materialized input names and paths must be unique".to_owned(),
                ));
            }
        }
        let directory = tempfile::Builder::new()
            .prefix("milkdrift-process-")
            .tempdir_in(&self.temporary_root)
            .map_err(|error| fs_error("create isolated execution directory", &error))?;
        set_private_directory_permissions(directory.path())?;
        let root = directory
            .path()
            .canonicalize()
            .map_err(|error| fs_error("canonicalize execution directory", &error))?;
        let mut materialized = BTreeMap::new();
        let mut total = 0_u64;
        for specification in inputs {
            let input = request
                .inputs()
                .iter()
                .find(|input| input.name() == specification.input_name())
                .ok_or_else(|| {
                    InvocationDataError::Rejected(format!(
                        "required invocation input '{}' is missing",
                        specification.input_name()
                    ))
                })?;
            let (bytes, _cause) = self.input_bytes(input, limits)?;
            let size = u64::try_from(bytes.len()).map_err(|_error| {
                InvocationDataError::Rejected("materialized input size overflow".to_owned())
            })?;
            if size > limits.max_file_bytes {
                return Err(InvocationDataError::Rejected(format!(
                    "input '{}' exceeds the per-file materialization bound",
                    input.name()
                )));
            }
            total = total.checked_add(size).ok_or_else(|| {
                InvocationDataError::Rejected("materialized byte accounting overflow".to_owned())
            })?;
            if total > limits.max_total_bytes {
                return Err(InvocationDataError::Rejected(
                    "selected inputs exceed the aggregate materialization bound".to_owned(),
                ));
            }
            let destination = create_regular_destination(&root, specification.relative_path())?;
            let mut file = destination.1;
            file.write_all(&bytes)
                .and_then(|()| file.sync_all())
                .map_err(|error| fs_error("write materialized input", &error))?;
            materialized.insert(input.name().to_owned(), destination.0);
        }
        Ok(Box::new(StoreMaterializedExecution {
            _directory: directory,
            root,
            inputs: materialized,
        }))
    }

    #[allow(clippy::too_many_arguments)]
    fn publish_file(
        &self,
        context: &AdapterExecutionContext,
        request: &InvocationRequest,
        workspace: &dyn MaterializedExecution,
        output_name: &str,
        relative_path: &Path,
        media_type: &str,
        limits: MaterializationLimits,
    ) -> Result<CapabilityArtifactReference, InvocationDataError> {
        let limits = limits.validate()?;
        validate_relative_path(
            relative_path,
            limits.max_path_bytes,
            limits.max_directory_depth,
        )?;
        let path = verified_regular_output(workspace.root(), relative_path)?;
        let metadata = fs::metadata(&path).map_err(|error| fs_error("inspect output", &error))?;
        if metadata.len() > limits.max_file_bytes {
            return Err(InvocationDataError::Rejected(
                "declared output exceeds the per-file publication bound".to_owned(),
            ));
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            if metadata.nlink() != 1 {
                return Err(InvocationDataError::Rejected(
                    "hard-linked output files are not publishable".to_owned(),
                ));
            }
        }
        let file = File::open(&path).map_err(|error| fs_error("open output", &error))?;
        let mut bytes = Vec::with_capacity(usize::try_from(metadata.len()).map_err(|_error| {
            InvocationDataError::Rejected("output size cannot fit this platform".to_owned())
        })?);
        file.take(limits.max_file_bytes.saturating_add(1))
            .read_to_end(&mut bytes)
            .map_err(|error| fs_error("read output", &error))?;
        if u64::try_from(bytes.len()) != Ok(metadata.len()) {
            return Err(InvocationDataError::Integrity(
                "output changed while it was being imported".to_owned(),
            ));
        }
        self.publish(context, request, output_name, media_type, &bytes, limits)
    }

    fn publish_bytes(
        &self,
        context: &AdapterExecutionContext,
        request: &InvocationRequest,
        output_name: &str,
        media_type: &str,
        bytes: &[u8],
        limits: MaterializationLimits,
    ) -> Result<CapabilityArtifactReference, InvocationDataError> {
        self.publish(
            context,
            request,
            output_name,
            media_type,
            bytes,
            limits.validate()?,
        )
    }
}

struct StoreMaterializedExecution {
    _directory: TempDir,
    root: PathBuf,
    inputs: BTreeMap<String, PathBuf>,
}

impl MaterializedExecution for StoreMaterializedExecution {
    fn root(&self) -> &Path {
        &self.root
    }

    fn input_path(&self, input_name: &str) -> Option<&Path> {
        self.inputs.get(input_name).map(PathBuf::as_path)
    }
}

fn durable_artifact_reference(
    reference: &CapabilityArtifactReference,
) -> Result<ArtifactReference, InvocationDataError> {
    let media_type = reference.media_type().ok_or_else(|| {
        InvocationDataError::Integrity("artifact input omits its exact media type".to_owned())
    })?;
    let size = reference.size_bytes().ok_or_else(|| {
        InvocationDataError::Integrity("artifact input omits its exact size".to_owned())
    })?;
    Ok(ArtifactReference::new(
        ArtifactId::new(reference.identity().to_owned())
            .map_err(|error| InvocationDataError::Integrity(error.to_string()))?,
        ContentDigest::from_hex(reference.digest())
            .map_err(|error| InvocationDataError::Integrity(error.to_string()))?,
        MediaType::new(media_type.to_owned())
            .map_err(|error| InvocationDataError::Integrity(error.to_string()))?,
        size,
    ))
}

fn capability_artifact_reference(
    reference: &ArtifactReference,
) -> Result<CapabilityArtifactReference, InvocationDataError> {
    CapabilityArtifactReference::new(
        reference.artifact().as_str().to_owned(),
        reference.digest().to_hex(),
        Some(reference.media_type().as_str().to_owned()),
        Some(reference.size_bytes()),
    )
    .map_err(|error| InvocationDataError::Publication(error.to_string()))
}

fn publication_hash(
    context: &AdapterExecutionContext,
    request: &InvocationRequest,
    output_name: &str,
    digest: ContentDigest,
) -> blake3::Hash {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"milkdrift.process-output-publication.v1\0");
    for component in [
        context.run().as_str().as_bytes(),
        context.revision().as_str().as_bytes(),
        context.node().as_str().as_bytes(),
        context.execution().as_str().as_bytes(),
        context.attempt().as_str().as_bytes(),
        request.invocation().as_str().as_bytes(),
        output_name.as_bytes(),
        digest.as_bytes(),
    ] {
        hasher.update(&(component.len() as u64).to_be_bytes());
        hasher.update(component);
    }
    hasher.finalize()
}

fn publication_causes(
    context: &AdapterExecutionContext,
    request: &InvocationRequest,
) -> Result<Vec<CausalReference>, InvocationDataError> {
    let mut causes = vec![
        external_cause("revision", context.revision().as_str())?,
        external_cause("node", context.node().as_str())?,
        external_cause("execution", context.execution().as_str())?,
        external_cause("attempt", context.attempt().as_str())?,
    ];
    for input in request.inputs() {
        let cause = match input.value() {
            InvocationValueReference::WorkspaceValue { identity, .. } => {
                let reference = serde_json::from_str(identity).map_err(|_error| {
                    InvocationDataError::Integrity(format!(
                        "workspace input '{}' is not an exact reference",
                        input.name()
                    ))
                })?;
                CausalReference::WorkspaceValue { reference }
            }
            InvocationValueReference::Artifact { reference } => CausalReference::Artifact {
                reference: durable_artifact_reference(reference)?,
            },
            InvocationValueReference::Inline { value } => {
                let bytes = serde_json::to_vec(value.value())
                    .map_err(|error| InvocationDataError::Integrity(error.to_string()))?;
                inline_cause(input.name(), &bytes)?
            }
        };
        causes.push(cause);
    }
    Ok(causes)
}

fn inline_cause(name: &str, bytes: &[u8]) -> Result<CausalReference, InvocationDataError> {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"milkdrift.inline-invocation-input.v1\0");
    hasher.update(&(name.len() as u64).to_be_bytes());
    hasher.update(name.as_bytes());
    hasher.update(&(bytes.len() as u64).to_be_bytes());
    hasher.update(bytes);
    Ok(CausalReference::External {
        source: CausalId::new(format!("inline:{}", hasher.finalize().to_hex()))
            .map_err(|error| InvocationDataError::Integrity(error.to_string()))?,
    })
}

fn external_cause(kind: &str, value: &str) -> Result<CausalReference, InvocationDataError> {
    let direct = format!("{kind}:{value}");
    let identity = if direct.len() <= 192 {
        direct
    } else {
        format!("{kind}:{}", blake3::hash(value.as_bytes()).to_hex())
    };
    Ok(CausalReference::External {
        source: CausalId::new(identity)
            .map_err(|error| InvocationDataError::Publication(error.to_string()))?,
    })
}

fn validate_output_name(value: &str) -> Result<(), InvocationDataError> {
    if value.is_empty()
        || value.len() > 128
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
    {
        return Err(InvocationDataError::Rejected(
            "output name must contain 1..=128 safe ASCII bytes".to_owned(),
        ));
    }
    Ok(())
}

fn validate_relative_path(
    path: &Path,
    max_path_bytes: usize,
    max_depth: usize,
) -> Result<(), InvocationDataError> {
    let encoded = path.to_str().ok_or_else(|| {
        InvocationDataError::Rejected("configured path is not valid UTF-8".to_owned())
    })?;
    if encoded.is_empty() || encoded.len() > max_path_bytes || encoded.contains('\0') {
        return Err(InvocationDataError::Rejected(
            "configured path violates its byte bound".to_owned(),
        ));
    }
    let mut depth = 0_usize;
    for component in path.components() {
        match component {
            Component::Normal(value) if !value.is_empty() => {
                depth = depth.saturating_add(1);
            }
            Component::Prefix(_)
            | Component::RootDir
            | Component::CurDir
            | Component::ParentDir
            | Component::Normal(_) => {
                return Err(InvocationDataError::Rejected(
                    "paths must be relative normal components without '.' or '..'".to_owned(),
                ));
            }
        }
    }
    if depth == 0 || depth > max_depth {
        return Err(InvocationDataError::Rejected(
            "configured path violates its directory-depth bound".to_owned(),
        ));
    }
    Ok(())
}

fn create_regular_destination(
    root: &Path,
    relative: &Path,
) -> Result<(PathBuf, File), InvocationDataError> {
    let parent = relative.parent().unwrap_or_else(|| Path::new(""));
    let mut current = root.to_path_buf();
    for component in parent.components() {
        let Component::Normal(component) = component else {
            return Err(InvocationDataError::Rejected(
                "input path contains an invalid component".to_owned(),
            ));
        };
        current.push(component);
        match fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
                return Err(InvocationDataError::Rejected(
                    "input path parent is not a plain directory".to_owned(),
                ));
            }
            Ok(_metadata) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                fs::create_dir(&current)
                    .map_err(|error| fs_error("create input directory", &error))?;
            }
            Err(error) => return Err(fs_error("inspect input directory", &error)),
        }
    }
    let destination = root.join(relative);
    let file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&destination)
        .map_err(|error| fs_error("create materialized input", &error))?;
    Ok((destination, file))
}

fn verified_regular_output(root: &Path, relative: &Path) -> Result<PathBuf, InvocationDataError> {
    let mut current = root.to_path_buf();
    for (index, component) in relative.components().enumerate() {
        let Component::Normal(component) = component else {
            return Err(InvocationDataError::Rejected(
                "output path contains an invalid component".to_owned(),
            ));
        };
        current.push(component);
        let metadata = fs::symlink_metadata(&current)
            .map_err(|error| fs_error("inspect declared output", &error))?;
        if metadata.file_type().is_symlink() {
            return Err(InvocationDataError::Rejected(
                "symlink outputs and symlink path components are not publishable".to_owned(),
            ));
        }
        let is_last = index + 1 == relative.components().count();
        if (!is_last && !metadata.is_dir()) || (is_last && !metadata.is_file()) {
            return Err(InvocationDataError::Rejected(
                "declared output is not a regular file beneath plain directories".to_owned(),
            ));
        }
    }
    let canonical = current
        .canonicalize()
        .map_err(|error| fs_error("canonicalize declared output", &error))?;
    if !canonical.starts_with(root) {
        return Err(InvocationDataError::Rejected(
            "declared output escapes the isolated execution root".to_owned(),
        ));
    }
    Ok(canonical)
}

#[cfg(unix)]
fn set_private_directory_permissions(path: &Path) -> Result<(), InvocationDataError> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
        .map_err(|error| fs_error("restrict execution directory", &error))
}

#[cfg(not(unix))]
fn set_private_directory_permissions(_path: &Path) -> Result<(), InvocationDataError> {
    Ok(())
}

fn fs_error(operation: &'static str, error: &std::io::Error) -> InvocationDataError {
    InvocationDataError::Filesystem(format!("{operation}: {:?}", error.kind()))
}
