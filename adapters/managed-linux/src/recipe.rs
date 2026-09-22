use crate::rejected;
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
        /// Exact operator-established provider billing contract.
        billing: milkdrift_model_provider::BillingTerms,
        /// Finite tokenizer/template/output contract used at ordinary serving admission.
        token_limits: milkdrift_model_provider::ModelTokenLimits,
    },
    /// A pinned llama-server image and exact local GGUF input.
    Owned {
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
        /// Configured context, bounded by the approved memory budget.
        context_tokens: u32,
        /// Operator-established tokenizer/template/output contract for this exact server/model.
        token_limits: milkdrift_model_provider::ModelTokenLimits,
        /// CPU worker threads.
        threads: u16,
        /// Explicit CPU or verified Vulkan device profile.
        backend: InferenceBackend,
    },
}

/// One maintained recipe with finite typed choices. Experimental workspace tool installs remain
/// mutable working data; promotion requires building and approving a new exact image input.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LinuxRecipe {
    /// Exact supported recipe schema.
    pub schema_version: u32,
    /// Operator-selected recipe name, separate from the maintained recipe family.
    pub name: ManagedName,
    /// Must be `slotbook-v1`; arbitrary recipe interpreters are not supported.
    pub family: String,
    /// Exact preloaded Git/Rust/build image. Floating tags never resolve at apply time.
    pub worker_image: String,
    /// Worker network choice.
    pub worker_network: WorkerNetwork,
    /// Shared memory limit in bytes; GPU reservations are not added to host RAM.
    pub memory_bytes: u64,
    /// CPU quota as a nonzero number of hundredths of a CPU.
    pub cpu_percent: u32,
    /// Process count limit including descendants.
    pub pids: u32,
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
        if bytes.len() > 65_536 {
            return Err(rejected("recipe exceeds 64 KiB"));
        }
        let value = milkdrift_contracts::parse_json_without_duplicates(bytes).map_err(rejected)?;
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
        if self.schema_version != 1
            || self.family != "slotbook-v1"
            || !exact_image(&self.worker_image)
            || !(268_435_456..=137_438_953_472).contains(&self.memory_bytes)
            || !(1..=12_800).contains(&self.cpu_percent)
            || !(16..=4096).contains(&self.pids)
            || !(1000..=3_600_000).contains(&self.task_timeout_ms)
            || !(1024..=1_048_576).contains(&self.output_bytes)
            || self.minimum_free_bytes == 0
        {
            return Err(rejected(
                "unsupported recipe family, unpinned image or invalid resource bounds",
            ));
        }
        match &self.model_service {
            ModelService::Disabled {} => {}
            ModelService::Attached {
                api_base,
                model_alias,
                billing,
                token_limits,
            } => {
                if matches!(billing, milkdrift_model_provider::BillingTerms::Unknown)
                    || matches!(
                        token_limits,
                        milkdrift_model_provider::ModelTokenLimits::Unknown
                    )
                {
                    return Err(rejected(
                        "managed model calls require explicit finite billing and token contracts",
                    ));
                }
                let url = url::Url::parse(api_base).map_err(rejected)?;
                if !matches!(url.scheme(), "http" | "https")
                    || !url.username().is_empty()
                    || url.password().is_some()
                    || url.query().is_some()
                    || url.fragment().is_some()
                    || url.host_str().is_none()
                    || api_base.len() > 2048
                {
                    return Err(rejected(
                        "attachment must be a credential-free HTTP(S) base",
                    ));
                }
                if model_alias.is_empty()
                    || model_alias.len() > 256
                    || model_alias.chars().any(char::is_control)
                {
                    return Err(rejected("attachment requires a bounded exact model alias"));
                }
            }
            ModelService::Owned {
                image,
                executable,
                model,
                model_digest,
                model_bytes,
                port,
                context_tokens,
                threads,
                backend,
                token_limits,
            } => {
                if matches!(
                    token_limits,
                    milkdrift_model_provider::ModelTokenLimits::Unknown
                ) {
                    return Err(rejected(
                        "managed model calls require an explicit finite token contract",
                    ));
                }
                if !exact_image(image)
                    || !safe_absolute(Path::new(executable))
                    || !safe_absolute(model)
                    || !milkdrift_contracts::is_canonical_blake3_digest(model_digest)
                    || *model_bytes == 0
                    || *model_bytes > 137_438_953_472
                    || *port < 1024
                    || !(512..=131_072).contains(context_tokens)
                    || !(1..=128).contains(threads)
                {
                    return Err(rejected("invalid pinned llama-server/model configuration"));
                }
                if let InferenceBackend::Vulkan { render_device } = backend {
                    let text = render_device.to_str().unwrap_or("");
                    if !text
                        .strip_prefix("/dev/dri/renderD")
                        .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
                    {
                        return Err(rejected(
                            "Vulkan profile requires exactly one render device",
                        ));
                    }
                }
            }
        }
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
    /// Exact approved recipe document paths. Startup freezes their normalized bytes.
    pub recipes: Vec<PathBuf>,
}
impl LinuxManagerConfig {
    /// Validate lexical paths without creating accounts, changing privileges or installing software.
    pub fn validate(&self) -> Result<(), ManagedError> {
        if !safe_absolute(&self.state_root)
            || !safe_absolute(&self.quadlet_directory)
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
                && prefix.len() <= 200
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
}
