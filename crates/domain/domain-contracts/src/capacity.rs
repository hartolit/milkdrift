//! Capacity descriptions for fixed-size, allocation-free execution buffers.

/// Identifies the bounded resource that could not satisfy an operation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum CapacityResource {
    /// Prompt or sequence token storage.
    Tokens,
    /// Vocabulary-sized model logits.
    Logits,
    /// Candidate token indices used during sampling.
    CandidateIndices,
    /// Repetition-history storage.
    RepetitionHistory,
    /// Incrementally decoded UTF-8 bytes.
    DecodeBytes,
    /// Accumulated text waiting for a consumer.
    OutputBytes,
    /// Runtime event records waiting for a consumer.
    OutputRecords,
    /// Backend-specific pre-allocated scratch storage.
    BackendScratch,
    /// Concurrent active requests.
    ActiveRequests,
    /// Concurrent active sequences.
    ActiveSequences,
    /// Prefill batch entries.
    PrefillBatch,
    /// Entries considered by the context planner.
    ContextEntries,
    /// Candidate indices used by the sampler.
    SamplingIndices,
    /// Vocabulary-sized sampling membership mask.
    SamplingMask,
    /// Nodes stored in an orchestration task graph.
    TaskNodes,
    /// Dependency edges stored in an orchestration task graph.
    TaskEdges,
}

/// Describes a checked fixed-capacity failure without allocating diagnostic text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CapacityExhausted {
    /// Resource whose bound was exceeded.
    pub resource: CapacityResource,
    /// Capacity required by the attempted operation.
    pub required: u64,
    /// Capacity available to the attempted operation.
    pub available: u64,
}

impl CapacityExhausted {
    /// Creates a capacity failure description.
    #[must_use]
    pub const fn new(resource: CapacityResource, required: u64, available: u64) -> Self {
        Self {
            resource,
            required,
            available,
        }
    }
}
