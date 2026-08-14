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
pub use verification::{
    CargoCommand, CommandPlanError, VerificationComponent, VerificationOperation, VerificationPlan,
    benchmark_command_plan, benchmark_command_plan_for_metadata, cuda_clippy_command_plan,
    cuda_clippy_command_plan_for_metadata, cuda_compile_command_plan,
    cuda_compile_command_plan_for_metadata, hardware_profile_command_plan,
    hardware_profile_command_plan_for_metadata, is_supported_portable_target,
    native_verification_plan, portable_command_plan, portable_command_plan_for_metadata,
    verification_component_plan, verification_component_plan_for_metadata,
};
