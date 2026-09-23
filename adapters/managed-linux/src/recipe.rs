use crate::rejected;
mod limits;
pub(crate) use limits::validate_timeout;
pub use limits::{ContainerLimits, ServiceTimeouts};

/// Current strict managed Linux recipe format. Earlier formats require explicit reconfiguration.
pub const LINUX_RECIPE_SCHEMA_VERSION: u32 = 2;
// Bounds the human-authored recipe and its durable deployment, not workload output.
pub(crate) const MAX_RECIPE_BYTES: usize = 65_536;
// One complete Linux 16-bit user/group ID space, disjoint from the manager identity.
pub(crate) const PRIVATE_USER_IDS: u64 = 65_536;
use milkdrift_capability::managed::{DataDisposition, ManagedName, RecipeReference};
use milkdrift_capability_host::managed::ManagedError;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};

/// Explicit worker networking. Unrestricted outbound access is never described as destination isolation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkerNetwork {
    /// Separate network namespace with no external connectivity.
    None,
    /// Rootless pasta networking with no host-loopback mapping; destinations are unrestricted.
    Outbound,
}

/// Supported inference mechanism, selected explicitly; CPU success supplies no GPU evidence.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum InferenceBackend {
    /// No passed devices.
    Cpu {},
    /// Exactly one operator-selected render node; no wildcard /dev mount.
    Vulkan {
        /// Canonical render device such as /dev/dri/renderD128.
        render_device: PathBuf,
        /// Positive number of layers to offload, selected for this model and device.
        gpu_layers: u32,
    },
}

/// Owned llama-server or an external endpoint attachment. Neither is an invocation lifetime.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ModelService {
    /// Bootstrap/build use needs no model.
    Disabled {},
    /// Owned elsewhere. Removal never sends stop/delete requests to this endpoint.
    Attached {
        /// Credential-free HTTP(S) API base ending at the API's /v1 path.
        api_base: String,
        /// Exact operator-declared API model identifier.
        model_alias: String,
        /// Transport and stream bounds validated by the existing model-provider owner.
        endpoint_limits: milkdrift_model_provider::EndpointLimits,
        /// Exact operator-established provider billing contract.
        billing: milkdrift_model_provider::BillingTerms,
        /// Finite tokenizer/template/output contract used at ordinary serving admission.
        token_limits: milkdrift_model_provider::ModelTokenLimits,
    },
    /// A pinned llama-server image and exact local GGUF input.
    Owned {
        /// Single API model identifier shared by the server and its published capability.
        model_alias: String,
        /// Transport and stream bounds validated by the existing model-provider owner.
        endpoint_limits: milkdrift_model_provider::EndpointLimits,
        /// Independent service container limits; worker limits are configured separately.
        limits: ContainerLimits,
        /// Preparation, startup and shutdown waits for this server/model combination.
        timeouts: ServiceTimeouts,
        /// Exact OCI reference or local sha256 image identity.
        image: String,
        /// Absolute image-internal server executable.
        executable: String,
        /// Shared operator-owned model file, mounted read-only and never deleted here.
        model: PathBuf,
        /// Exact BLAKE3 digest of the model bytes.
        model_digest: String,
        /// Exact model byte length.
        model_bytes: u64,
        /// Host loopback-only port, without a worker route to the manager's loopback interface.
        port: u16,
        /// Configured llama-server context; container memory is enforced separately.
        context_tokens: u32,
        /// Operator-established tokenizer/template/output contract for this exact server/model.
        token_limits: milkdrift_model_provider::ModelTokenLimits,
        /// CPU worker threads.
        threads: u16,
        /// Explicit CPU or verified Vulkan device profile.
        backend: InferenceBackend,
    },
}

/// Approved working environment with finite typed choices, independent of application content. Experimental workspace tool installs remain
/// mutable working data; promotion requires building and approving a new exact image input.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LinuxRecipe {
    /// Exact supported recipe schema.
    pub schema_version: u32,
    /// Operator-selected recipe name.
    pub name: ManagedName,
    /// Exact preloaded worker image. Floating tags never resolve at apply time.
    pub worker_image: String,
    /// Worker network choice.
    pub worker_network: WorkerNetwork,
    /// Independent limits for the temporary worker container.
    pub worker_limits: ContainerLimits,
    /// Maximum single worker command duration.
    pub task_timeout_ms: u64,
    /// Maximum combined worker stdout/stderr bytes; overflow is a failure.
    pub output_bytes: u64,
    /// Minimum observed free storage before preparation.
    pub minimum_free_bytes: u64,
    /// Initial data preservation policy.
    pub data_disposition: DataDisposition,
    /// Optional model service or attachment.
    pub model_service: ModelService,
}

impl LinuxRecipe {
    /// Decode strict bounded JSON through the production reader.
    pub fn from_json(bytes: &[u8]) -> Result<Self, ManagedError> {
        if bytes.len() > MAX_RECIPE_BYTES {
            return Err(rejected("recipe exceeds 64 KiB"));
        }
        let value = milkdrift_contracts::parse_json_without_duplicates(bytes).map_err(rejected)?;
        if value.get("schema_version").and_then(|v| v.as_u64())
            != Some(u64::from(LINUX_RECIPE_SCHEMA_VERSION))
        {
            return Err(rejected(format!(
                "unsupported recipe schema; expected {LINUX_RECIPE_SCHEMA_VERSION}"
            )));
        }
        let recipe: Self = serde_json::from_value(value).map_err(rejected)?;
        recipe.validate()?;
        Ok(recipe)
    }
    /// Exact reference used by prepare/apply/update; source formatting is immaterial.
    pub fn reference(&self) -> Result<RecipeReference, ManagedError> {
        self.validate()?;
        let bytes = milkdrift_contracts::canonical_json_bytes(
            self,
            milkdrift_contracts::JsonLimits {
                maximum_depth: 12,
                maximum_string_bytes: 4096,
                maximum_key_bytes: 128,
                maximum_container_items: 64,
            },
        )
        .map_err(|e| rejected(format!("{e:?}")))?;
        Ok(RecipeReference {
            name: self.name.clone(),
            digest: crate::digest(bytes),
        })
    }
    fn validate(&self) -> Result<(), ManagedError> {
        if self.schema_version != LINUX_RECIPE_SCHEMA_VERSION {
            return Err(rejected(format!(
                "unsupported recipe schema {}; expected {}",
                self.schema_version, LINUX_RECIPE_SCHEMA_VERSION
            )));
        }
        if !exact_image(&self.worker_image) {
            return Err(rejected(
                "worker_image must be an exact preloaded sha256 identity",
            ));
        }
        self.worker_limits.validate("worker_limits")?;
        limits::validate_timeout(self.task_timeout_ms, "task_timeout_ms")?;
        if self.output_bytes == 0 {
            return Err(rejected("output_bytes must be positive"));
        }
        crate::worker::output_artifact_limit(self.output_bytes)?;
        if self.minimum_free_bytes == 0 || self.minimum_free_bytes > i64::MAX as u64 {
            return Err(rejected(
                "minimum_free_bytes must be positive signed-64-bit bytes",
            ));
        }
        if let ModelService::Owned {
            image,
            executable,
            model,
            model_digest,
            model_bytes,
            port,
            context_tokens,
            threads,
            backend,
            limits,
            timeouts,
            model_alias,
            ..
        } = &self.model_service
        {
            limits.validate("model_service.limits")?;
            timeouts.validate()?;
            // --alias accepts a comma-separated list. This contract publishes exactly one name,
            // with no shell/systemd interpolation or argument syntax.
            if model_alias.is_empty()
                || !model_alias.as_bytes()[0].is_ascii_alphanumeric()
                || !model_alias.bytes().all(|b| {
                    b.is_ascii_alphanumeric() || matches!(b, b'/' | b':' | b'.' | b'_' | b'-')
                })
            {
                return Err(rejected(
                    "model_service.model_alias must be one identifier using letters, digits, / : . _ -, beginning with a letter or digit",
                ));
            }
            if !exact_image(image) {
                return Err(rejected(
                    "model_service.image must be an exact sha256 identity",
                ));
            }
            if !safe_absolute(Path::new(executable)) {
                return Err(rejected(
                    "model_service.executable must be a safe absolute container path",
                ));
            }
            if !safe_absolute(model) {
                return Err(rejected(
                    "model_service.model must be a safe absolute host path",
                ));
            }
            if !milkdrift_contracts::is_canonical_blake3_digest(model_digest) {
                return Err(rejected(
                    "model_service.model_digest must be a canonical BLAKE3 digest",
                ));
            }
            if *model_bytes == 0 || *model_bytes > i64::MAX as u64 {
                return Err(rejected(
                    "model_service.model_bytes must be positive signed-64-bit bytes",
                ));
            }
            if *port < 1024 {
                return Err(rejected(
                    "model_service.port must be an unprivileged port (1024 or higher)",
                ));
            }
            if *context_tokens == 0 || *context_tokens > i32::MAX as u32 {
                return Err(rejected(
                    "model_service.context_tokens must fit llama-server's positive signed context size",
                ));
            }
            if *threads == 0 {
                return Err(rejected("model_service.threads must be positive"));
            }
            if let InferenceBackend::Vulkan {
                render_device,
                gpu_layers,
            } = backend
            {
                if !render_device
                    .to_str()
                    .unwrap_or("")
                    .strip_prefix("/dev/dri/renderD")
                    .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
                {
                    return Err(rejected(
                        "model_service.backend.render_device must identify exactly one /dev/dri/renderD device",
                    ));
                }
                if *gpu_layers == 0 || *gpu_layers > i32::MAX as u32 {
                    return Err(rejected(
                        "model_service.backend.gpu_layers must be a positive signed layer count",
                    ));
                }
            }
            self.worker_limits
                .memory_bytes
                .checked_add(limits.memory_bytes)
                .ok_or_else(|| rejected("combined worker and service memory overflows bytes"))?;
        }
        // The provider owns endpoint, transport and accounting validation. Use the same constructor
        // here and at publication so accepted configuration cannot be ignored later.
        crate::model::profile_for_service(&self.model_service, &self.name, 1)?;
        Ok(())
    }
}

/// Host-only bootstrap configuration. Workers receive none of these paths or credentials.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LinuxManagerConfig {
    /// Existing private manager data directory, outside every worker mount.
    pub state_root: PathBuf,
    /// Existing private rootless Quadlet search directory.
    pub quadlet_directory: PathBuf,
    /// Rootless systemd user unit directory for exact container-ID cleanup drop-ins.
    pub systemd_directory: PathBuf,
    /// Exact approved recipe document paths. Startup freezes their normalized bytes.
    pub recipes: Vec<PathBuf>,
}
impl LinuxManagerConfig {
    /// Validate lexical paths without creating accounts, changing privileges or installing software.
    pub fn validate(&self) -> Result<(), ManagedError> {
        if !safe_absolute(&self.state_root)
            || !safe_absolute(&self.quadlet_directory)
            || !safe_absolute(&self.systemd_directory)
            || self.recipes.is_empty()
            || self.recipes.len() > 32
            || self.recipes.iter().any(|p| !safe_absolute(p))
            || self.recipes.iter().collect::<BTreeSet<_>>().len() != self.recipes.len()
        {
            return Err(rejected(
                "manager requires bounded unique absolute private paths without interpolation",
            ));
        }
        Ok(())
    }
}

pub(crate) fn safe_absolute(path: &Path) -> bool {
    path.is_absolute()
        && path.to_str().is_some_and(|s| {
            s.len() <= 4096
                && s.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'/' | b'-' | b'_' | b'.'))
        })
        && path.components().all(|c| {
            matches!(
                c,
                std::path::Component::RootDir | std::path::Component::Normal(_)
            )
        })
        && !path
            .to_str()
            .unwrap_or("")
            .split('/')
            .any(|c| c == ".." || c == ".")
}
pub(crate) fn exact_image(value: &str) -> bool {
    let Some((prefix, hex)) = value.rsplit_once("sha256:") else {
        return false;
    };
    hex.len() == 64
        && hex
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        && (prefix.is_empty()
            || (prefix
                .as_bytes()
                .first()
                .is_some_and(u8::is_ascii_alphanumeric)
                && prefix.bytes().filter(|b| *b == b'@').count() == 1
                && prefix.ends_with('@')
                && prefix.bytes().all(|b| {
                    b.is_ascii_lowercase()
                        || b.is_ascii_digit()
                        || matches!(b, b'/' | b'-' | b'_' | b'.' | b'@' | b':')
                })))
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Deployment {
    pub recipe: LinuxRecipe,
    pub installation: ManagedName,
    pub generation: u64,
    pub unit: String,
    pub volume_prefix: String,
    pub manager_root: PathBuf,
    pub quadlet_directory: PathBuf,
    pub systemd_directory: PathBuf,
}
