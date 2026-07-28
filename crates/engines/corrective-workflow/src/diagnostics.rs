//! Typed validation diagnostics and deterministic normalization.

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
    value.and_then(|text| {
        let trimmed = text.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_owned())
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
