//! Application-owned selection and model-reporting vocabulary.

/// Local execution engine reported by E1.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ApplicationEngine {
    /// Candle local execution.
    Candle,
}

/// Model artifact source reported by E1.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ApplicationSource {
    /// Immutable artifacts resolved through Hugging Face Hub.
    HuggingFaceHub,
}

/// Application-owned identity of one local execution device.
///
/// CUDA ordinals are process-local backend selectors rather than permanent
/// hardware identities.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ApplicationDevice {
    /// Host CPU execution.
    Cpu,
    /// One CUDA device selected by its backend-visible ordinal.
    Cuda {
        /// Zero-based CUDA ordinal.
        ordinal: u32,
    },
}

impl ApplicationDevice {
    pub(crate) fn base_label(self) -> String {
        match self {
            Self::Cpu => "CPU".to_owned(),
            Self::Cuda { ordinal } => format!("CUDA {ordinal}"),
        }
    }
}

/// CUDA compute capability translated into application-owned vocabulary.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ApplicationComputeCapability {
    /// Compute-capability major version.
    pub major: u32,
    /// Compute-capability minor version.
    pub minor: u32,
}

/// Stable reason that a selected application device cannot currently load a model.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ApplicationDeviceUnavailableReason {
    /// The binary was built without the selected device implementation.
    SupportNotCompiled,
    /// Bounded device initialization or fact discovery failed.
    DiscoveryFailed,
}

/// One frontend-neutral device choice and its latest observed facts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ApplicationDeviceSummary {
    device: ApplicationDevice,
    label: String,
    available: bool,
    unavailable_reason: Option<ApplicationDeviceUnavailableReason>,
    total_memory_bytes: Option<u64>,
    available_memory_bytes: Option<u64>,
    compute_capability: Option<ApplicationComputeCapability>,
}

impl ApplicationDeviceSummary {
    pub(crate) fn cpu() -> Self {
        Self {
            device: ApplicationDevice::Cpu,
            label: "CPU".to_owned(),
            available: true,
            unavailable_reason: None,
            total_memory_bytes: None,
            available_memory_bytes: None,
            compute_capability: None,
        }
    }

    pub(crate) fn discovered(
        device: ApplicationDevice,
        label: String,
        total_memory_bytes: Option<u64>,
        available_memory_bytes: Option<u64>,
        compute_capability: Option<ApplicationComputeCapability>,
    ) -> Self {
        Self {
            device,
            label,
            available: true,
            unavailable_reason: None,
            total_memory_bytes,
            available_memory_bytes,
            compute_capability,
        }
    }

    pub(crate) fn unavailable(
        device: ApplicationDevice,
        reason: ApplicationDeviceUnavailableReason,
    ) -> Self {
        Self {
            device,
            label: device.base_label(),
            available: false,
            unavailable_reason: Some(reason),
            total_memory_bytes: None,
            available_memory_bytes: None,
            compute_capability: None,
        }
    }

    /// Returns the stable application device identity.
    #[must_use]
    pub const fn device(&self) -> ApplicationDevice {
        self.device
    }

    /// Returns a presentation-ready CPU or CUDA ordinal label.
    #[must_use]
    pub const fn label(&self) -> &str {
        self.label.as_str()
    }

    /// Returns whether the latest bounded probe found the device available.
    #[must_use]
    pub const fn available(&self) -> bool {
        self.available
    }

    /// Returns the normalized reason an unavailable selected device cannot be used.
    #[must_use]
    pub const fn unavailable_reason(&self) -> Option<ApplicationDeviceUnavailableReason> {
        self.unavailable_reason
    }

    /// Returns total device-local memory reported during discovery.
    #[must_use]
    pub const fn total_memory_bytes(&self) -> Option<u64> {
        self.total_memory_bytes
    }

    /// Returns the point-in-time available device-local memory observation.
    #[must_use]
    pub const fn available_memory_bytes(&self) -> Option<u64> {
        self.available_memory_bytes
    }

    /// Returns the translated CUDA compute capability when available.
    #[must_use]
    pub const fn compute_capability(&self) -> Option<ApplicationComputeCapability> {
        self.compute_capability
    }
}

/// Stable category for one cold-path device-discovery failure.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ApplicationDeviceDiscoveryFailureKind {
    /// The application supplied an invalid device identity.
    InvalidConfiguration,
    /// The compiled backend does not support the requested device.
    Unsupported,
    /// Driver or device initialization failed.
    Initialization,
    /// Discovery failed outside the more specific stable categories.
    Other,
}

/// Application-owned diagnostic for one failed bounded device probe.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ApplicationDeviceDiscoveryFailure {
    device: ApplicationDevice,
    kind: ApplicationDeviceDiscoveryFailureKind,
    message: String,
}

impl ApplicationDeviceDiscoveryFailure {
    pub(crate) fn new(
        device: ApplicationDevice,
        kind: ApplicationDeviceDiscoveryFailureKind,
        message: String,
    ) -> Self {
        Self {
            device,
            kind,
            message,
        }
    }

    /// Returns the device whose probe failed.
    #[must_use]
    pub const fn device(&self) -> ApplicationDevice {
        self.device
    }

    /// Returns the stable failure category.
    #[must_use]
    pub const fn kind(&self) -> ApplicationDeviceDiscoveryFailureKind {
        self.kind
    }

    /// Returns the owned cold-path diagnostic.
    #[must_use]
    pub const fn message(&self) -> &str {
        self.message.as_str()
    }
}

/// Model serialization format reported by E1.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ApplicationModelFormat {
    /// Safetensors shards plus model configuration.
    Safetensors,
}

/// User-visible Hugging Face model selection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelSelection {
    repository: String,
    revision: String,
}

impl ModelSelection {
    /// Creates a normalized Hugging Face repository and revision selection.
    #[must_use]
    pub fn new(repository: impl Into<String>, revision: impl Into<String>) -> Self {
        Self {
            repository: repository.into().trim().to_owned(),
            revision: revision.into().trim().to_owned(),
        }
    }

    /// Returns the normalized Hugging Face repository.
    #[must_use]
    pub const fn repository(&self) -> &str {
        self.repository.as_str()
    }

    /// Returns the normalized requested revision.
    #[must_use]
    pub const fn revision(&self) -> &str {
        self.revision.as_str()
    }

    pub(crate) fn into_parts(self) -> (String, String) {
        (self.repository, self.revision)
    }
}

/// Scalar category used by application-level configuration metadata or execution facts.
///
/// The containing field defines provenance. This type alone never means that every
/// serialized tensor has the same scalar representation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ApplicationScalarType {
    /// IEEE-754 32-bit floating point.
    F32,
    /// IEEE-754 16-bit floating point.
    F16,
    /// Brain floating point.
    Bf16,
}

/// Immutable Hugging Face artifact identity retained across resolution and loading.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImmutableModelIdentity {
    repository: String,
    commit: String,
}

impl ImmutableModelIdentity {
    pub(crate) fn new(repository: impl Into<String>, commit: impl Into<String>) -> Self {
        Self {
            repository: repository.into().trim().to_owned(),
            commit: commit.into().trim().to_owned(),
        }
    }

    /// Returns the repository whose immutable commit was resolved.
    #[must_use]
    pub const fn repository(&self) -> &str {
        self.repository.as_str()
    }

    /// Returns the immutable Hub commit identifier.
    #[must_use]
    pub const fn commit(&self) -> &str {
        self.commit.as_str()
    }
}
