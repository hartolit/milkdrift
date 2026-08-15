//! Typed Cargo workspace metadata and maintained target inventories.

mod benchmark;
mod hardware;
mod inventory;
mod metadata;
mod role;

pub(crate) const METADATA_NAMESPACE: &str = "milkdrift";
pub(crate) const CUDA_FEATURE: &str = "cuda";
pub(crate) const CUDA_HARDWARE_FEATURE: &str = "cuda-hardware-tests";
pub(crate) const CUDA_HARDWARE_TARGET: &str = "cuda_hardware";

pub(crate) use benchmark::benchmark_inventory;
pub(crate) use hardware::{
    HardwareSuite, cuda_hardware_target_inventory, hardware_suite_inventory,
    inspect_cuda_hardware_target,
};
pub(crate) use inventory::{
    WorkspaceInventoryIssue, cuda_feature_package_inventory, domain_package_inventory,
    workspace_package_inventory,
};
pub(crate) use metadata::load_metadata;
pub use role::Role;
pub(crate) use role::{
    cuda_provider, package_role, relative_manifest, role_location_is_compatible,
};
