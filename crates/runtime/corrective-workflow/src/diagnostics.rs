//! Typed validation diagnostics and deterministic normalization.

use super::output::{
    OutputSinkError, VALIDATION_VERDICT_BYTES, diagnostic_fields_size, normalized_report_size,
    try_clone_text,
};

/// Severity assigned to a validation finding.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum DiagnosticSeverity {
    /// Informational finding that does not itself indicate failure.
    Information,
    /// Warning that may require attention.
    Warning,
    /// Error that prevents validation from passing.
    Error,
}

/// Optional source location associated with a diagnostic.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct DiagnosticLocation {
    /// Optional source path supplied by the validator.
    pub path: Option<String>,
    /// Optional one-based source line.
    pub line: Option<u32>,
    /// Optional one-based source column.
    pub column: Option<u32>,
}

/// Typed, unnormalized finding returned by a validation port.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawDiagnostic {
    /// Finding severity.
    pub severity: DiagnosticSeverity,
    /// Optional validator-defined diagnostic code.
    pub code: Option<String>,
    /// Human-readable diagnostic message.
    pub message: String,
    /// Optional source location.
    pub location: Option<DiagnosticLocation>,
}

/// Stable normalized finding consumed by later workflow stages.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Diagnostic {
    /// Finding severity.
    pub severity: DiagnosticSeverity,
    /// Trimmed optional validator-defined diagnostic code.
    pub code: Option<String>,
    /// Trimmed message with runs of whitespace collapsed to one ASCII space.
    pub message: String,
    /// Normalized optional source location.
    pub location: Option<DiagnosticLocation>,
}

/// Deterministic validation decision.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ValidationVerdict {
    /// The checked artifact satisfies the validator.
    Passed,
    /// The checked artifact has findings but validation completed normally.
    Rejected,
}

/// Raw typed result returned by a validation port.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ValidationReport {
    /// Validation decision.
    pub verdict: ValidationVerdict,
    /// Typed findings in validator-provided order.
    pub diagnostics: Vec<RawDiagnostic>,
}

/// Deterministically ordered and deduplicated validation result.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NormalizedValidationReport {
    /// Validation decision preserved from the raw report.
    pub verdict: ValidationVerdict,
    /// Sorted, deduplicated normalized findings.
    pub diagnostics: Vec<Diagnostic>,
}

/// Normalizes one typed validation report without parsing vendor-formatted text.
///
/// Messages have surrounding whitespace removed and internal whitespace runs
/// collapsed. Optional codes and paths are trimmed and discarded when empty.
/// Findings are then sorted by their typed fields and deduplicated.
#[must_use]
pub fn normalize_validation_report(report: &ValidationReport) -> NormalizedValidationReport {
    let mut diagnostics: Vec<Diagnostic> = report
        .diagnostics
        .iter()
        .map(normalize_diagnostic)
        .collect();
    diagnostics.sort();
    diagnostics.dedup();
    NormalizedValidationReport {
        verdict: report.verdict,
        diagnostics,
    }
}

pub fn normalize_validation_report_bounded(
    report: &ValidationReport,
    maximum_bytes: u64,
) -> Result<NormalizedValidationReport, OutputSinkError> {
    if maximum_bytes < VALIDATION_VERDICT_BYTES {
        return Err(OutputSinkError::CapacityExceeded {
            required: VALIDATION_VERDICT_BYTES,
            maximum: maximum_bytes,
        });
    }

    let allocation_bound = normalized_candidates_size(report)?;
    let mut diagnostics = Vec::new();
    diagnostics
        .try_reserve_exact(report.diagnostics.len())
        .map_err(|_| OutputSinkError::AllocationFailed {
            required: allocation_bound,
        })?;
    for raw in &report.diagnostics {
        diagnostics.push(try_normalize_diagnostic(raw, allocation_bound)?);
    }
    diagnostics.sort();
    diagnostics.dedup();

    let normalized = NormalizedValidationReport {
        verdict: report.verdict,
        diagnostics,
    };
    let required = normalized_report_size(&normalized).ok_or(OutputSinkError::SizeOverflow)?;
    if required > maximum_bytes {
        return Err(OutputSinkError::CapacityExceeded {
            required,
            maximum: maximum_bytes,
        });
    }
    Ok(normalized)
}

fn normalized_candidates_size(report: &ValidationReport) -> Result<u64, OutputSinkError> {
    report
        .diagnostics
        .iter()
        .try_fold(VALIDATION_VERDICT_BYTES, |total, raw| {
            let code = normalized_optional_text(raw.code.as_deref());
            let message_bytes = u64::try_from(collapsed_whitespace_len(&raw.message)?)
                .map_err(|_| OutputSinkError::SizeOverflow)?;
            let location = raw.location.as_ref().map(|location| {
                (
                    normalized_optional_text(location.path.as_deref()),
                    location.line,
                    location.column,
                )
            });
            let diagnostic_bytes = diagnostic_fields_size(code, message_bytes, location)
                .ok_or(OutputSinkError::SizeOverflow)?;
            total
                .checked_add(diagnostic_bytes)
                .ok_or(OutputSinkError::SizeOverflow)
        })
}

fn normalize_diagnostic(raw: &RawDiagnostic) -> Diagnostic {
    Diagnostic {
        severity: raw.severity,
        code: normalize_optional_text(raw.code.as_deref()),
        message: collapse_whitespace(&raw.message),
        location: raw.location.as_ref().map(normalize_location),
    }
}

fn normalize_location(location: &DiagnosticLocation) -> DiagnosticLocation {
    DiagnosticLocation {
        path: normalize_optional_text(location.path.as_deref()),
        line: location.line,
        column: location.column,
    }
}

fn normalize_optional_text(value: Option<&str>) -> Option<String> {
    normalized_optional_text(value).map(str::to_owned)
}

fn normalized_optional_text(value: Option<&str>) -> Option<&str> {
    value.and_then(|text| {
        let trimmed = text.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed)
        }
    })
}

fn collapse_whitespace(value: &str) -> String {
    let mut normalized = String::new();
    for word in value.split_whitespace() {
        if !normalized.is_empty() {
            normalized.push(' ');
        }
        normalized.push_str(word);
    }
    normalized
}

fn try_normalize_diagnostic(
    raw: &RawDiagnostic,
    required: u64,
) -> Result<Diagnostic, OutputSinkError> {
    Ok(Diagnostic {
        severity: raw.severity,
        code: normalized_optional_text(raw.code.as_deref())
            .map(|code| try_clone_text(code, required))
            .transpose()?,
        message: try_collapse_whitespace(&raw.message, required)?,
        location: raw
            .location
            .as_ref()
            .map(|location| try_normalize_location(location, required))
            .transpose()?,
    })
}

fn try_normalize_location(
    location: &DiagnosticLocation,
    required: u64,
) -> Result<DiagnosticLocation, OutputSinkError> {
    Ok(DiagnosticLocation {
        path: normalized_optional_text(location.path.as_deref())
            .map(|path| try_clone_text(path, required))
            .transpose()?,
        line: location.line,
        column: location.column,
    })
}

fn try_collapse_whitespace(value: &str, required: u64) -> Result<String, OutputSinkError> {
    let length = collapsed_whitespace_len(value)?;
    let mut normalized = String::new();
    normalized
        .try_reserve_exact(length)
        .map_err(|_| OutputSinkError::AllocationFailed { required })?;
    for word in value.split_whitespace() {
        if !normalized.is_empty() {
            normalized.push(' ');
        }
        normalized.push_str(word);
    }
    Ok(normalized)
}

fn collapsed_whitespace_len(value: &str) -> Result<usize, OutputSinkError> {
    let mut length = 0_usize;
    for word in value.split_whitespace() {
        if length != 0 {
            length = length.checked_add(1).ok_or(OutputSinkError::SizeOverflow)?;
        }
        length = length
            .checked_add(word.len())
            .ok_or(OutputSinkError::SizeOverflow)?;
    }
    Ok(length)
}

#[cfg(test)]
mod tests {
    use super::{
        DiagnosticLocation, DiagnosticSeverity, RawDiagnostic, ValidationReport, ValidationVerdict,
        normalize_validation_report, normalize_validation_report_bounded,
    };

    #[test]
    fn bounded_normalization_matches_sort_and_dedup_semantics() {
        let duplicate = RawDiagnostic {
            severity: DiagnosticSeverity::Warning,
            code: Some(" W1 ".to_owned()),
            message: " alpha\u{2003}beta ".to_owned(),
            location: Some(DiagnosticLocation {
                path: Some(" src/lib.rs ".to_owned()),
                line: Some(2),
                column: None,
            }),
        };
        let report = ValidationReport {
            verdict: ValidationVerdict::Rejected,
            diagnostics: vec![
                duplicate.clone(),
                RawDiagnostic {
                    severity: DiagnosticSeverity::Information,
                    code: Some(" ".to_owned()),
                    message: " zeta ".to_owned(),
                    location: None,
                },
                duplicate,
            ],
        };

        assert_eq!(
            normalize_validation_report_bounded(&report, 1_024),
            Ok(normalize_validation_report(&report))
        );
    }
}
