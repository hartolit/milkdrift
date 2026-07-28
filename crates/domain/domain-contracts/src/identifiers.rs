//! Strongly typed identifiers used across architectural boundaries.

macro_rules! define_id {
    ($name:ident, $doc:literal) => {
        #[doc = $doc]
        #[repr(transparent)]
        #[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(u64);

        impl $name {
            /// Creates an identifier from its stable numeric representation.
            #[must_use]
            pub const fn new(value: u64) -> Self {
                Self(value)
            }

            /// Returns the stable numeric representation.
            #[must_use]
            pub const fn get(self) -> u64 {
                self.0
            }
        }
    };
}

define_id!(ModelId, "Logical identity of a model known to the runtime.");
define_id!(
    ModelGeneration,
    "Generation counter used to reject stale model handles."
);
define_id!(
    SequenceId,
    "Identity of one model-specific inference sequence."
);
define_id!(RequestId, "Identity of one generation request.");
define_id!(TaskId, "Identity of one orchestration task.");
define_id!(
    ArtifactId,
    "Identity of one immutable model or workflow artifact."
);
define_id!(
    BackendId,
    "Identity of a compiled inference backend implementation."
);
define_id!(DeviceId, "Identity of a backend-visible execution device.");

/// Numeric token identifier interpreted by a model tokenizer and vocabulary.
#[repr(transparent)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TokenId(u32);

impl TokenId {
    /// Creates a token identifier.
    #[must_use]
    pub const fn new(value: u32) -> Self {
        Self(value)
    }

    /// Returns the numeric token value.
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }
}

/// Non-owning runtime handle for a specific loaded generation of a model.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ModelHandle {
    /// Logical model identity.
    pub id: ModelId,
    /// Runtime generation used to reject handles retained across unload and reload.
    pub generation: ModelGeneration,
}

impl ModelHandle {
    /// Creates a model handle.
    #[must_use]
    pub const fn new(id: ModelId, generation: ModelGeneration) -> Self {
        Self { id, generation }
    }
}
