//! Layered workspace architecture validation.

mod policy;
mod report;
mod traversal;

pub use policy::{DependencyKind, Layer};
pub use report::{ValidationError, ValidationReport, Violation};
pub use traversal::validate_workspace;
