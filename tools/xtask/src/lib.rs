//! Workspace architecture, repository hygiene, and exact verification planning.

#![forbid(unsafe_code)]

mod architecture;
mod hygiene;
mod verification;
mod workspace;

pub use architecture::{
    DependencyKind, Layer, ValidationError, ValidationReport, Violation, validate_workspace,
};
pub use hygiene::{HygieneError, HygieneReport, HygieneViolation, validate_repository_hygiene};
pub use verification::{BenchmarkPlanError, CargoCommand, benchmark_command_plan};
