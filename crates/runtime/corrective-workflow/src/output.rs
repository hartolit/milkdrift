//! Executor-owned bounded output storage.

use std::error::Error;
use std::fmt::{self, Display, Formatter};

use super::{
    Diagnostic, DiagnosticLocation, NormalizedValidationReport, RawDiagnostic, ValidationReport,
    ValidationVerdict,
};

pub const VALIDATION_VERDICT_BYTES: u64 = 1;
const DIAGNOSTIC_SEVERITY_BYTES: u64 = 1;
const OPTION_TAG_BYTES: u64 = 1;
const U32_PAYLOAD_BYTES: u64 = 4;

/// Stable failure produced while growing an executor-owned output sink.
///
/// Sink failures are sticky: after the first failure, all later appends return
/// the same failure without modifying the accepted output.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutputSinkError {
    /// The append would exceed the sink's declared logical byte maximum.
    CapacityExceeded {
        /// Logical bytes required if the append were accepted.
        required: u64,
        /// Declared logical byte maximum.
        maximum: u64,
    },
    /// Logical output byte accounting overflowed or was not representable.
    SizeOverflow,
    /// Fallible reservation for logically admissible output storage failed.
    AllocationFailed {
        /// Logical bytes required if the append were accepted.
        required: u64,
    },
}

impl Display for OutputSinkError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::CapacityExceeded { required, maximum } => {
                write!(
                    formatter,
                    "output requires {required} bytes but permits {maximum}"
                )
            }
            Self::SizeOverflow => formatter.write_str("output byte accounting overflowed"),
            Self::AllocationFailed { required } => write!(
                formatter,
                "allocation failed while reserving output requiring {required} bytes"
            ),
        }
    }
}

impl Error for OutputSinkError {}

/// Executor-owned text storage bounded by a stage output contract.
///
/// Each append checks the complete post-append UTF-8 length and performs a
/// fallible reservation before changing the text. An unsuccessful append is
/// atomic and never truncates either the existing text or the supplied chunk.
#[derive(Debug)]
pub struct BoundedTextSink {
    text: String,
    maximum_bytes: u64,
    bytes_written: u64,
    failure: Option<OutputSinkError>,
}

impl BoundedTextSink {
    /// Creates an empty text sink with the declared UTF-8 byte maximum.
    #[must_use]
    pub const fn new(maximum_bytes: u64) -> Self {
        Self {
            text: String::new(),
            maximum_bytes,
            bytes_written: 0,
            failure: None,
        }
    }

    /// Appends one complete UTF-8 chunk without truncation.
    ///
    /// # Errors
    ///
    /// Returns a sticky [`OutputSinkError`] before modifying the accepted text
    /// when accounting, capacity, or fallible allocation fails.
    pub fn append(&mut self, chunk: &str) -> Result<(), OutputSinkError> {
        if let Some(failure) = self.failure {
            return Err(failure);
        }
        let Ok(additional) = u64::try_from(chunk.len()) else {
            return self.fail(OutputSinkError::SizeOverflow);
        };
        let Some(required) = self.bytes_written.checked_add(additional) else {
            return self.fail(OutputSinkError::SizeOverflow);
        };
        if required > self.maximum_bytes {
            return self.fail(OutputSinkError::CapacityExceeded {
                required,
                maximum: self.maximum_bytes,
            });
        }
        if self.text.try_reserve(chunk.len()).is_err() {
            return self.fail(OutputSinkError::AllocationFailed { required });
        }
        self.text.push_str(chunk);
        self.bytes_written = required;
        Ok(())
    }

    /// Returns the accepted text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.text
    }

    /// Returns the accepted UTF-8 byte count.
    #[must_use]
    pub const fn bytes_written(&self) -> u64 {
        self.bytes_written
    }

    /// Returns the declared UTF-8 byte maximum.
    #[must_use]
    pub const fn maximum_bytes(&self) -> u64 {
        self.maximum_bytes
    }

    /// Returns the remaining logical byte allowance.
    #[must_use]
    pub const fn remaining_bytes(&self) -> u64 {
        self.maximum_bytes.saturating_sub(self.bytes_written)
    }

    /// Returns whether no text has been accepted.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.bytes_written == 0
    }

    /// Returns the first sink failure, if an append failed.
    #[must_use]
    pub const fn failure(&self) -> Option<OutputSinkError> {
        self.failure
    }

    pub(super) fn finish(self) -> Result<String, OutputSinkError> {
        match self.failure {
            Some(failure) => Err(failure),
            None => Ok(self.text),
        }
    }

    const fn fail<T>(&mut self, failure: OutputSinkError) -> Result<T, OutputSinkError> {
        self.failure = Some(failure);
        Err(failure)
    }
}

/// Executor-owned raw validation diagnostics bounded by a stage output contract.
///
/// Accounting starts with one byte for the returned [`ValidationVerdict`]. Each
/// accepted diagnostic accounts for severity and option tags, string UTF-8
/// payloads, and optional line/column payloads. Consequently, both diagnostic
/// count and all string growth consume the same declared byte maximum.
#[derive(Debug)]
pub struct BoundedDiagnosticsSink {
    diagnostics: Vec<RawDiagnostic>,
    maximum_bytes: u64,
    bytes_written: u64,
    failure: Option<OutputSinkError>,
}

impl BoundedDiagnosticsSink {
    /// Creates an empty diagnostics sink and accounts for its verdict byte.
    ///
    /// # Errors
    ///
    /// Returns [`OutputSinkError::CapacityExceeded`] when `maximum_bytes` cannot
    /// hold the structured verdict.
    pub const fn new(maximum_bytes: u64) -> Result<Self, OutputSinkError> {
        if maximum_bytes < VALIDATION_VERDICT_BYTES {
            return Err(OutputSinkError::CapacityExceeded {
                required: VALIDATION_VERDICT_BYTES,
                maximum: maximum_bytes,
            });
        }
        Ok(Self {
            diagnostics: Vec::new(),
            maximum_bytes,
            bytes_written: VALIDATION_VERDICT_BYTES,
            failure: None,
        })
    }

    /// Copies one diagnostic into executor-owned storage without truncation.
    ///
    /// The complete structured post-append size is checked before any diagnostic
    /// is committed. Vector and string storage use fallible reservation, and a
    /// failed append leaves the accepted diagnostic sequence unchanged.
    ///
    /// # Errors
    ///
    /// Returns a sticky [`OutputSinkError`] when accounting, capacity, or
    /// fallible allocation fails.
    pub fn append(&mut self, diagnostic: &RawDiagnostic) -> Result<(), OutputSinkError> {
        if let Some(failure) = self.failure {
            return Err(failure);
        }
        let Some(diagnostic_bytes) = raw_diagnostic_size(diagnostic) else {
            return self.fail(OutputSinkError::SizeOverflow);
        };
        let Some(required) = self.bytes_written.checked_add(diagnostic_bytes) else {
            return self.fail(OutputSinkError::SizeOverflow);
        };
        if required > self.maximum_bytes {
            return self.fail(OutputSinkError::CapacityExceeded {
                required,
                maximum: self.maximum_bytes,
            });
        }
        if self.diagnostics.try_reserve(1).is_err() {
            return self.fail(OutputSinkError::AllocationFailed { required });
        }
        let owned = match try_clone_raw_diagnostic(diagnostic, required) {
            Ok(owned) => owned,
            Err(failure) => return self.fail(failure),
        };
        self.diagnostics.push(owned);
        self.bytes_written = required;
        Ok(())
    }

    /// Returns the accepted diagnostics in validator-provided order.
    #[must_use]
    pub fn diagnostics(&self) -> &[RawDiagnostic] {
        &self.diagnostics
    }

    /// Returns accounted bytes, including the verdict byte.
    #[must_use]
    pub const fn bytes_written(&self) -> u64 {
        self.bytes_written
    }

    /// Returns the declared structured byte maximum.
    #[must_use]
    pub const fn maximum_bytes(&self) -> u64 {
        self.maximum_bytes
    }

    /// Returns the remaining logical byte allowance.
    #[must_use]
    pub const fn remaining_bytes(&self) -> u64 {
        self.maximum_bytes.saturating_sub(self.bytes_written)
    }

    /// Returns whether no diagnostics have been accepted.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.diagnostics.is_empty()
    }

    /// Returns the first sink failure, if an append failed.
    #[must_use]
    pub const fn failure(&self) -> Option<OutputSinkError> {
        self.failure
    }

    pub(super) fn finish(
        self,
        verdict: ValidationVerdict,
    ) -> Result<ValidationReport, OutputSinkError> {
        match self.failure {
            Some(failure) => Err(failure),
            None => Ok(ValidationReport {
                verdict,
                diagnostics: self.diagnostics,
            }),
        }
    }

    const fn fail<T>(&mut self, failure: OutputSinkError) -> Result<T, OutputSinkError> {
        self.failure = Some(failure);
        Err(failure)
    }
}

pub fn raw_report_size(report: &ValidationReport) -> Option<u64> {
    report
        .diagnostics
        .iter()
        .try_fold(VALIDATION_VERDICT_BYTES, |total, diagnostic| {
            checked_add(total, raw_diagnostic_size(diagnostic)?)
        })
}

pub fn normalized_report_size(report: &NormalizedValidationReport) -> Option<u64> {
    report
        .diagnostics
        .iter()
        .try_fold(VALIDATION_VERDICT_BYTES, |total, diagnostic| {
            checked_add(total, diagnostic_size(diagnostic)?)
        })
}

fn raw_diagnostic_size(diagnostic: &RawDiagnostic) -> Option<u64> {
    diagnostic_fields_size(
        diagnostic.code.as_deref(),
        string_size(&diagnostic.message)?,
        diagnostic
            .location
            .as_ref()
            .map(|location| (location.path.as_deref(), location.line, location.column)),
    )
}

fn diagnostic_size(diagnostic: &Diagnostic) -> Option<u64> {
    diagnostic_fields_size(
        diagnostic.code.as_deref(),
        string_size(&diagnostic.message)?,
        diagnostic
            .location
            .as_ref()
            .map(|location| (location.path.as_deref(), location.line, location.column)),
    )
}

pub fn diagnostic_fields_size(
    code: Option<&str>,
    message_bytes: u64,
    location: Option<(Option<&str>, Option<u32>, Option<u32>)>,
) -> Option<u64> {
    let total = checked_add(DIAGNOSTIC_SEVERITY_BYTES, optional_string_size(code)?)?;
    let total = checked_add(total, message_bytes)?;
    checked_add(total, optional_location_fields_size(location)?)
}

pub fn try_clone_text(value: &str, required: u64) -> Result<String, OutputSinkError> {
    let mut owned = String::new();
    owned
        .try_reserve_exact(value.len())
        .map_err(|_| OutputSinkError::AllocationFailed { required })?;
    owned.push_str(value);
    Ok(owned)
}

fn try_clone_raw_diagnostic(
    diagnostic: &RawDiagnostic,
    required: u64,
) -> Result<RawDiagnostic, OutputSinkError> {
    Ok(RawDiagnostic {
        severity: diagnostic.severity,
        code: try_clone_optional_text(diagnostic.code.as_deref(), required)?,
        message: try_clone_text(&diagnostic.message, required)?,
        location: diagnostic
            .location
            .as_ref()
            .map(|location| try_clone_location(location, required))
            .transpose()?,
    })
}

fn try_clone_location(
    location: &DiagnosticLocation,
    required: u64,
) -> Result<DiagnosticLocation, OutputSinkError> {
    Ok(DiagnosticLocation {
        path: try_clone_optional_text(location.path.as_deref(), required)?,
        line: location.line,
        column: location.column,
    })
}

fn try_clone_optional_text(
    value: Option<&str>,
    required: u64,
) -> Result<Option<String>, OutputSinkError> {
    value.map(|text| try_clone_text(text, required)).transpose()
}

fn optional_string_size(value: Option<&str>) -> Option<u64> {
    let payload = value.map_or(Some(0), string_size)?;
    checked_add(OPTION_TAG_BYTES, payload)
}

fn optional_location_fields_size(
    location: Option<(Option<&str>, Option<u32>, Option<u32>)>,
) -> Option<u64> {
    let Some((path, line, column)) = location else {
        return Some(OPTION_TAG_BYTES);
    };
    let total = checked_add(OPTION_TAG_BYTES, optional_string_size(path)?)?;
    let total = checked_add(total, optional_u32_size(line))?;
    checked_add(total, optional_u32_size(column))
}

const fn optional_u32_size(value: Option<u32>) -> u64 {
    if value.is_some() {
        OPTION_TAG_BYTES + U32_PAYLOAD_BYTES
    } else {
        OPTION_TAG_BYTES
    }
}

fn string_size(value: &str) -> Option<u64> {
    u64::try_from(value.len()).ok()
}

const fn checked_add(left: u64, right: u64) -> Option<u64> {
    left.checked_add(right)
}
