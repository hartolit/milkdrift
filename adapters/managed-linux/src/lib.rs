//! Prepare and maintain approved working environments through rootless Podman and user Quadlets.
//!
//! The host resource owner commits intent and use claims. This adapter checks exact platform
//! identities, creates bounded definitions and asks systemd to supervise persistent services.
//! Workers receive only an installation-owned volume, never manager paths or engine sockets.
//! [`LinuxRecipe`] is operator configuration, not a general container administration language.

mod command;
mod model;
mod platform;
mod recipe;
mod service;
mod units;
mod worker;
pub use model::ManagedModelAdapter;
pub use platform::LinuxManagedPlatform;
pub use recipe::{
    ContainerLimits, InferenceBackend, LINUX_RECIPE_SCHEMA_VERSION, LinuxManagerConfig,
    LinuxRecipe, ModelService, ServiceTimeouts, WorkerNetwork,
};
pub use worker::ManagedWorkerAdapter;

use milkdrift_capability_host::managed::ManagedError;
fn rejected(value: impl std::fmt::Display) -> ManagedError {
    ManagedError::Rejected(milkdrift_contracts::truncate_utf8(&value.to_string(), 512).to_owned())
}
fn platform_error(value: impl std::fmt::Display) -> ManagedError {
    ManagedError::Platform(milkdrift_contracts::truncate_utf8(&value.to_string(), 512).to_owned())
}
fn digest(value: impl AsRef<[u8]>) -> String {
    format!("b3_{}", blake3::hash(value.as_ref()))
}

pub use platform::ProtectedServiceRecipe;
