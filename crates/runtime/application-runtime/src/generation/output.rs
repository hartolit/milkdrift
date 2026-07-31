use domain_contracts::{FinishReason, RequestId, YieldReason};
use host_runtime::{TextOutputBatch, TextOutputRecordKind};

/// Compact state payload published beside decoded text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ApplicationOutputState {
    /// E0 yielded without completing the request.
    Yielded(YieldReason),
    /// Generation ended and sequence cleanup is beginning.
    Terminal(GenerationTerminalKind),
    /// Explicit sequence cleanup failed but remains retryable.
    CleanupPending,
    /// Automatic cleanup attempts are exhausted.
    CleanupExhausted,
    /// Sequence cleanup completed and request accounting was released.
    Released(GenerationTerminalKind),
}

/// Allocation-free terminal classification used in pulled output records.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GenerationTerminalKind {
    /// Generation completed with a stable finish reason.
    Finished(FinishReason),
    /// Generation failed; the detailed normalized failure is available as an event/state summary.
    Failed,
}

/// Absolute UTF-8 byte range within the application output stream.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ApplicationTextRange {
    /// Inclusive absolute byte position.
    pub start: u64,
    /// Number of UTF-8 bytes in the range.
    pub length: usize,
}

/// One request-scoped decoded output record.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ApplicationOutputRecordKind {
    /// UTF-8 text committed to the batch byte storage.
    Text(ApplicationTextRange),
    /// Application generation or cleanup state.
    State(ApplicationOutputState),
}

/// One request-scoped decoded output record.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ApplicationOutputRecord {
    /// Request that produced the record.
    pub request_id: RequestId,
    /// Text or state payload.
    pub kind: ApplicationOutputRecordKind,
}

/// Borrowed decoded output batch exposed without host-runtime implementation types.
pub struct ApplicationOutputBatch<'a> {
    inner: TextOutputBatch<'a, ApplicationOutputState>,
}

impl<'a> ApplicationOutputBatch<'a> {
    pub(super) const fn new(inner: TextOutputBatch<'a, ApplicationOutputState>) -> Self {
        Self { inner }
    }

    /// Returns the absolute byte cursor before this batch.
    #[must_use]
    pub const fn start(&self) -> u64 {
        self.inner.start.get()
    }

    /// Returns the absolute byte cursor after this batch.
    #[must_use]
    pub const fn end(&self) -> u64 {
        self.inner.end.get()
    }

    /// Returns the contiguous UTF-8 bytes retained by this batch.
    #[must_use]
    pub const fn bytes(&self) -> &'a [u8] {
        let bytes: &'a [u8] = self.inner.bytes;
        bytes
    }

    /// Iterates over copied frontend-neutral record descriptions.
    pub fn records(&self) -> impl Iterator<Item = ApplicationOutputRecord> + '_ {
        self.inner.records.iter().map(|record| {
            let kind = match record.kind {
                TextOutputRecordKind::Text(range) => {
                    ApplicationOutputRecordKind::Text(ApplicationTextRange {
                        start: range.start.get(),
                        length: range.length,
                    })
                }
                TextOutputRecordKind::State(state) => ApplicationOutputRecordKind::State(state),
            };
            ApplicationOutputRecord {
                request_id: record.request_id,
                kind,
            }
        })
    }

    /// Resolves one text record to its UTF-8 fragment when it belongs to this batch.
    #[must_use]
    pub fn text_for(&self, record: ApplicationOutputRecord) -> Option<&'a str> {
        let ApplicationOutputRecordKind::Text(range) = record.kind else {
            return None;
        };
        let offset = range.start.checked_sub(self.start())?;
        let offset = usize::try_from(offset).ok()?;
        let end = offset.checked_add(range.length)?;
        let bytes: &'a [u8] = self.inner.bytes.get(offset..end)?;
        std::str::from_utf8(bytes).ok()
    }
}
