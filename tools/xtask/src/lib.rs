//! Workspace architecture and repository hygiene validation.

#![forbid(unsafe_code)]

mod architecture;
mod hygiene;

pub use architecture::{
    DependencyKind, Layer, ValidationError, ValidationReport, Violation, validate_workspace,
};
pub use hygiene::{HygieneError, HygieneReport, HygieneViolation, validate_repository_hygiene};
